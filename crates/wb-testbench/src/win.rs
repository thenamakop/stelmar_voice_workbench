//! Windows calls for the testbench: a Direct3D 11 compute load on a chosen GPU adapter, and a
//! Media Foundation decode of a WAV file (the decoder Windows' own players use).
//!
//! The only module in `wb-testbench` that may use `unsafe` (docs/UNSAFE.md). Each block holds
//! one call and says why it is sound.
#![allow(unsafe_code)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_UNORDERED_ACCESS, D3D11_BUFFER_DESC, D3D11_CREATE_DEVICE_FLAG, D3D11_QUERY_DESC,
    D3D11_QUERY_EVENT, D3D11_RESOURCE_MISC_BUFFER_STRUCTURED, D3D11_SDK_VERSION,
    D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Buffer, ID3D11ComputeShader, ID3D11Device,
    ID3D11DeviceContext, ID3D11Query, ID3D11UnorderedAccessView,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1};
use windows::Win32::Media::MediaFoundation::{
    IMFSample, IMFSourceReader, MF_SOURCE_READER_FIRST_AUDIO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READERF_ERROR, MF_VERSION, MFCreateSourceReaderFromURL, MFSTARTUP_FULL, MFShutdown,
    MFStartup,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::core::{BOOL, HSTRING, PCSTR};

/// A GPU adapter as DXGI lists it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Adapter {
    pub index: u32,
    pub name: String,
    pub vendor_id: u32,
    pub dedicated_mb: u64,
    pub shared_mb: u64,
}

fn adapter_name(desc: &[u16; 128]) -> String {
    let end = desc.iter().position(|&c| c == 0).unwrap_or(desc.len());
    String::from_utf16_lossy(&desc[..end])
}

fn factory() -> Result<IDXGIFactory1> {
    // SAFETY: CreateDXGIFactory1 takes no pointers from us and returns an owned COM interface.
    let f = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }?;
    Ok(f)
}

/// Every DXGI adapter.
pub fn adapters() -> Result<Vec<(Adapter, IDXGIAdapter1)>> {
    let factory = factory()?;
    let mut out = Vec::new();
    for index in 0..16u32 {
        // SAFETY: `factory` is a live IDXGIFactory1; an index past the end returns an error.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        // SAFETY: `adapter` is a live IDXGIAdapter1 returned just above.
        let desc = unsafe { adapter.GetDesc1() }?;
        out.push((
            Adapter {
                index,
                name: adapter_name(&desc.Description),
                vendor_id: desc.VendorId,
                dedicated_mb: desc.DedicatedVideoMemory as u64 / (1 << 20),
                shared_mb: desc.SharedSystemMemory as u64 / (1 << 20),
            },
            adapter,
        ));
    }
    Ok(out)
}

const SHADER: &str = r"
RWStructuredBuffer<float> outp : register(u0);
[numthreads(256, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    float a = (float)id.x * 0.000001;
    float b = 1.0001;
    [loop] for (uint i = 0; i < 4096; i++) { a = a * b + 0.0001; b = b * 0.99999 + 0.00001; }
    outp[id.x] = a + b;
}
";

const GROUPS: u32 = 1024;

struct GpuLoad {
    ctx: ID3D11DeviceContext,
    shader: ID3D11ComputeShader,
    uav: Option<ID3D11UnorderedAccessView>,
    query: ID3D11Query,
}

fn compile_shader() -> Result<Vec<u8>> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: the source pointer and length describe SHADER, a static string; the entry point and
    // target are NUL-terminated literals; both out-pointers point to live Options.
    let hr = unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(c"main".as_ptr().cast()),
            PCSTR(c"cs_5_0".as_ptr().cast()),
            0,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(e) = hr {
        let msg = errors
            .map(|b| blob_bytes(&b))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        bail!("shader compile failed: {e} {msg}");
    }
    let code = code.ok_or_else(|| anyhow!("shader compile returned no code"))?;
    Ok(blob_bytes(&code))
}

fn blob_bytes(blob: &ID3DBlob) -> Vec<u8> {
    // SAFETY: `blob` is a live ID3DBlob; GetBufferPointer returns its buffer.
    let ptr = unsafe { blob.GetBufferPointer() };
    // SAFETY: as above; GetBufferSize returns the buffer's length in bytes.
    let len = unsafe { blob.GetBufferSize() };
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: ptr and len describe the blob's buffer, which lives as long as `blob` (borrowed for
    // this call); the bytes are copied out before returning.
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    bytes.to_vec()
}

impl GpuLoad {
    fn new(adapter: &IDXGIAdapter1) -> Result<Self> {
        let mut device: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        // SAFETY: `adapter` is live; the feature-level slice and the two out-pointers are valid for
        // the call; a null software module is required with D3D_DRIVER_TYPE_UNKNOWN.
        unsafe {
            D3D11CreateDevice(
                adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
        }
        .context("D3D11CreateDevice")?;
        let device = device.ok_or_else(|| anyhow!("no D3D11 device"))?;
        let ctx = ctx.ok_or_else(|| anyhow!("no D3D11 context"))?;
        let bytecode = compile_shader()?;
        let mut shader: Option<ID3D11ComputeShader> = None;
        // SAFETY: `bytecode` is compiled cs_5_0 code that outlives the call; the out-pointer is live.
        unsafe { device.CreateComputeShader(&bytecode, None, Some(&mut shader)) }
            .context("CreateComputeShader")?;
        let shader = shader.ok_or_else(|| anyhow!("no compute shader"))?;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: GROUPS * 256 * 4,
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_UNORDERED_ACCESS.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_BUFFER_STRUCTURED.0 as u32,
            StructureByteStride: 4,
        };
        let mut buffer: Option<ID3D11Buffer> = None;
        // SAFETY: `desc` describes a valid structured buffer and lives for the call; no initial data.
        unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) }.context("CreateBuffer")?;
        let buffer = buffer.ok_or_else(|| anyhow!("no buffer"))?;
        let mut uav: Option<ID3D11UnorderedAccessView> = None;
        // SAFETY: `buffer` is a live structured buffer; a null description views all of it.
        unsafe { device.CreateUnorderedAccessView(&buffer, None, Some(&mut uav)) }
            .context("CreateUnorderedAccessView")?;
        let qdesc = D3D11_QUERY_DESC {
            Query: D3D11_QUERY_EVENT,
            MiscFlags: 0,
        };
        let mut query: Option<ID3D11Query> = None;
        // SAFETY: `qdesc` is a valid event-query description that lives for the call.
        unsafe { device.CreateQuery(&qdesc, Some(&mut query)) }.context("CreateQuery")?;
        let query = query.ok_or_else(|| anyhow!("no query"))?;
        Ok(Self {
            ctx,
            shader,
            uav,
            query,
        })
    }

    /// Runs one dispatch and waits for the GPU to finish it.
    fn dispatch_and_wait(&self, stop: &AtomicBool) -> Result<()> {
        // SAFETY: the shader belongs to this context's device; no class instances.
        unsafe { self.ctx.CSSetShader(&self.shader, None) };
        // SAFETY: one UAV slot, pointing at a live view of this device; no initial counts.
        unsafe {
            self.ctx
                .CSSetUnorderedAccessViews(0, 1, Some(&self.uav), None)
        };
        // SAFETY: shader and UAV are bound above; the group count is within D3D11 limits.
        unsafe { self.ctx.Dispatch(GROUPS, 1, 1) };
        // SAFETY: the query was created on this device.
        unsafe { self.ctx.End(&self.query) };
        // SAFETY: Flush takes no arguments.
        unsafe { self.ctx.Flush() };
        loop {
            let mut done = BOOL(0);
            // SAFETY: `done` is a live BOOL of the size passed, which an event query writes.
            // windows-rs maps S_FALSE (not ready) to Ok, so readiness is read from `done`.
            let r = unsafe {
                self.ctx.GetData(
                    &self.query,
                    Some(std::ptr::from_mut(&mut done).cast()),
                    std::mem::size_of::<BOOL>() as u32,
                    0,
                )
            };
            r.context("GetData")?;
            if done.as_bool() || stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_micros(500));
        }
    }
}

/// Keeps the first adapter whose name contains `name_part` busy until `stop` is set. Returns the
/// adapter used, the number of dispatches and the GPU-busy fraction of wall time.
pub fn gpu_load(name_part: &str, stop: &AtomicBool) -> Result<(Adapter, u64, f64)> {
    let needle = name_part.to_lowercase();
    let (adapter, handle) = adapters()?
        .into_iter()
        .find(|(a, _)| a.name.to_lowercase().contains(&needle))
        .ok_or_else(|| anyhow!("no GPU adapter whose name contains {name_part:?}"))?;
    let load = GpuLoad::new(&handle)?;
    let start = Instant::now();
    let mut dispatches = 0u64;
    while !stop.load(Ordering::Relaxed) {
        load.dispatch_and_wait(stop)?;
        dispatches += 1;
    }
    Ok((adapter, dispatches, start.elapsed().as_secs_f64()))
}

/// An audio session on a playback endpoint: which process is (or was) playing into it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioSession {
    pub endpoint: String,
    pub pid: u32,
    /// The session is playing now (AudioSessionStateActive).
    pub active: bool,
}

/// The default playback device for each Windows role: console (most apps and system sounds),
/// multimedia and communications (chat apps, some notification sounds). They can be different
/// devices, and any of them being the virtual cable lets sounds reach it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DefaultRender {
    pub console: Option<String>,
    pub multimedia: Option<String>,
    pub communications: Option<String>,
}

pub fn default_render_devices() -> Result<DefaultRender> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::{
        ERole, IMMDeviceEnumerator, MMDeviceEnumerator, eCommunications, eConsole, eMultimedia,
        eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};

    let _com = ComThread::start()?;
    // SAFETY: creates the system's device enumerator; COM is initialised on this thread.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .context("MMDeviceEnumerator")?;
    let name_for = |role: ERole| -> Option<String> {
        // SAFETY: `enumerator` is live; the data-flow and role are defined constants.
        let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, role) }.ok()?;
        // SAFETY: `device` is live; read-only access to its properties.
        let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
        // SAFETY: `store` is live and the key is a valid static PROPERTYKEY.
        unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
            .ok()
            .map(|v| v.to_string())
    };
    Ok(DefaultRender {
        console: name_for(eConsole),
        multimedia: name_for(eMultimedia),
        communications: name_for(eCommunications),
    })
}

/// Audio sessions on every active playback endpoint whose name contains `name_part`. The gate
/// uses it to show that nothing but its own player feeds the virtual cable.
pub fn render_sessions(name_part: &str) -> Result<Vec<AudioSession>> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::{
        AudioSessionStateActive, DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2,
        IMMDeviceEnumerator, MMDeviceEnumerator, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};
    use windows::core::Interface;

    let _com = ComThread::start()?;
    let needle = name_part.to_lowercase();
    // SAFETY: creates the system's device enumerator; COM is initialised on this thread.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .context("MMDeviceEnumerator")?;
    // SAFETY: `enumerator` is live; the flags are defined constants.
    let devices = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
        .context("EnumAudioEndpoints")?;
    // SAFETY: `devices` is a live collection.
    let count = unsafe { devices.GetCount() }?;
    let mut out = Vec::new();
    for i in 0..count {
        // SAFETY: `i` is below the collection's count.
        let device = unsafe { devices.Item(i) }?;
        // SAFETY: `device` is live; read-only access to its properties.
        let store = unsafe { device.OpenPropertyStore(STGM_READ) }?;
        // SAFETY: `store` is live and the key is a valid static PROPERTYKEY.
        let name = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
            .map(|v| v.to_string())
            .unwrap_or_default();
        if !name.to_lowercase().contains(&needle) {
            continue;
        }
        // SAFETY: `device` is live; IAudioSessionManager2 is a documented interface of render endpoints.
        let manager: IAudioSessionManager2 =
            unsafe { device.Activate(CLSCTX_ALL, None) }.context("IAudioSessionManager2")?;
        // SAFETY: `manager` is live.
        let sessions = unsafe { manager.GetSessionEnumerator() }?;
        // SAFETY: `sessions` is live.
        let n = unsafe { sessions.GetCount() }?;
        for j in 0..n {
            // SAFETY: `j` is below the enumerator's count.
            let control = unsafe { sessions.GetSession(j) }?;
            let control2: IAudioSessionControl2 = control.cast()?;
            // SAFETY: `control2` is live.
            let pid = unsafe { control2.GetProcessId() }.unwrap_or(0);
            // SAFETY: `control2` is live.
            let state = unsafe { control2.GetState() }?;
            out.push(AudioSession {
                endpoint: name.clone(),
                pid,
                active: state == AudioSessionStateActive,
            });
        }
    }
    Ok(out)
}

/// COM for the calling thread (without Media Foundation). When the thread already has COM in
/// another mode (cpal initialises it), that initialisation is used and left alone.
struct ComThread {
    owns: bool,
}

impl ComThread {
    fn start() -> Result<Self> {
        use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        // SAFETY: initialises COM for this thread; balanced by CoUninitialize in Drop when this
        // call is the one that initialised it.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_ok() {
            Ok(Self { owns: true })
        } else if hr == RPC_E_CHANGED_MODE {
            Ok(Self { owns: false })
        } else {
            Err(anyhow!("CoInitializeEx: {hr:?}"))
        }
    }
}

impl Drop for ComThread {
    fn drop(&mut self) {
        if self.owns {
            // SAFETY: balances the successful CoInitializeEx in `start` on this thread.
            unsafe { CoUninitialize() };
        }
    }
}

// ---- Raw WASAPI capture (the second reference) and endpoint mute / volume ----------------------

/// Which side of the machine an endpoint is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Capture,
    Render,
}

/// An active audio endpoint whose friendly name contains `name_part` (case-insensitive).
fn find_endpoint(
    flow: Flow,
    name_part: &str,
) -> Result<(windows::Win32::Media::Audio::IMMDevice, String)> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, IMMDeviceEnumerator, MMDeviceEnumerator, eCapture, eRender,
    };
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, STGM_READ};

    let needle = name_part.to_lowercase();
    // SAFETY: creates the system's device enumerator; COM is initialised on this thread by the caller.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .context("MMDeviceEnumerator")?;
    let data_flow = if flow == Flow::Capture {
        eCapture
    } else {
        eRender
    };
    // SAFETY: `enumerator` is live; the flags are defined constants.
    let devices = unsafe { enumerator.EnumAudioEndpoints(data_flow, DEVICE_STATE_ACTIVE) }
        .context("EnumAudioEndpoints")?;
    // SAFETY: `devices` is a live collection.
    let count = unsafe { devices.GetCount() }?;
    for i in 0..count {
        // SAFETY: `i` is below the collection's count.
        let device = unsafe { devices.Item(i) }?;
        // SAFETY: `device` is live; read-only access to its properties.
        let store = unsafe { device.OpenPropertyStore(STGM_READ) }?;
        // SAFETY: `store` is live and the key is a valid static PROPERTYKEY.
        let name = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }
            .map(|v| v.to_string())
            .unwrap_or_default();
        if name.to_lowercase().contains(&needle) {
            return Ok((device, name));
        }
    }
    bail!("no active audio endpoint whose name contains {name_part:?}")
}

fn endpoint_volume(
    device: &windows::Win32::Media::Audio::IMMDevice,
) -> Result<windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume> {
    use windows::Win32::System::Com::CLSCTX_ALL;
    // SAFETY: `device` is live; IAudioEndpointVolume is a documented interface of every endpoint.
    unsafe { device.Activate(CLSCTX_ALL, None) }.context("IAudioEndpointVolume")
}

/// Reads (and, with `set`, changes) an endpoint's mute switch. Returns (endpoint name, muted now).
pub fn endpoint_mute(flow: Flow, name_part: &str, set: Option<bool>) -> Result<(String, bool)> {
    let _com = ComThread::start()?;
    let (device, name) = find_endpoint(flow, name_part)?;
    let vol = endpoint_volume(&device)?;
    if let Some(m) = set {
        // SAFETY: `vol` is live; a null event-context GUID is allowed.
        unsafe { vol.SetMute(m, std::ptr::null()) }.context("SetMute")?;
    }
    // SAFETY: `vol` is live.
    let muted = unsafe { vol.GetMute() }.context("GetMute")?;
    Ok((name, muted.as_bool()))
}

/// Reads (and, with `set`, changes) an endpoint's master volume, 0.0 to 1.0. Returns (name, level now).
pub fn endpoint_level(flow: Flow, name_part: &str, set: Option<f32>) -> Result<(String, f32)> {
    let _com = ComThread::start()?;
    let (device, name) = find_endpoint(flow, name_part)?;
    let vol = endpoint_volume(&device)?;
    if let Some(l) = set {
        // SAFETY: `vol` is live; a null event-context GUID is allowed; the level is clamped to 0..1.
        unsafe { vol.SetMasterVolumeLevelScalar(l.clamp(0.0, 1.0), std::ptr::null()) }
            .context("SetMasterVolumeLevelScalar")?;
    }
    // SAFETY: `vol` is live.
    let level =
        unsafe { vol.GetMasterVolumeLevelScalar() }.context("GetMasterVolumeLevelScalar")?;
    Ok((name, level))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    F32,
    I16,
    I24,
    I32,
}

/// Turns one packet of little-endian PCM bytes into f32 samples in -1.0..1.0.
fn convert_samples(kind: SampleKind, bytes: &[u8], out: &mut [f32]) {
    match kind {
        SampleKind::F32 => {
            for (o, c) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
                *o = f32::from_le_bytes(*c);
            }
        }
        SampleKind::I16 => {
            for (o, c) in out.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *o = f32::from(i16::from_le_bytes(*c)) / 32_768.0;
            }
        }
        SampleKind::I24 => {
            for (o, c) in out.iter_mut().zip(bytes.as_chunks::<3>().0) {
                let v = i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8;
                *o = v as f32 / 8_388_608.0;
            }
        }
        SampleKind::I32 => {
            for (o, c) in out.iter_mut().zip(bytes.as_chunks::<4>().0) {
                *o = i32::from_le_bytes(*c) as f32 / 2_147_483_648.0;
            }
        }
    }
}

/// One packet from the capture client, as interleaved 32-bit float samples.
pub struct Packet {
    pub samples: Vec<f32>,
    pub frames: u32,
    /// AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY: Windows says data was lost before this packet.
    pub discontinuity: bool,
}

/// Raw WASAPI shared-mode, event-driven capture of one endpoint: no cpal in the path. Use it on the
/// thread that opened it.
pub struct WasapiCapture {
    _com: ComThread,
    client: windows::Win32::Media::Audio::IAudioClient,
    capture: windows::Win32::Media::Audio::IAudioCaptureClient,
    event: windows::Win32::Foundation::HANDLE,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    kind: SampleKind,
    block_align: usize,
}

impl WasapiCapture {
    pub fn open(name_part: &str) -> Result<Self> {
        use windows::Win32::Media::Audio::{
            AUDCLNT_SHAREMODE_SHARED, IAudioCaptureClient, IAudioClient, WAVEFORMATEX,
            WAVEFORMATEXTENSIBLE,
        };
        use windows::Win32::System::Com::{CLSCTX_ALL, CoTaskMemFree};
        use windows::Win32::System::Threading::CreateEventW;
        use windows::core::PCWSTR;

        // AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        const EVENTCALLBACK: u32 = 0x0004_0000;
        let com = ComThread::start()?;
        let (device, device_name) = find_endpoint(Flow::Capture, name_part)?;
        // SAFETY: `device` is live; IAudioClient is a documented interface of every endpoint.
        let client: IAudioClient =
            unsafe { device.Activate(CLSCTX_ALL, None) }.context("IAudioClient")?;
        // SAFETY: `client` is live; the call returns a CoTaskMem-allocated format that is freed below.
        let pwfx: *mut WAVEFORMATEX = unsafe { client.GetMixFormat() }.context("GetMixFormat")?;
        if pwfx.is_null() {
            bail!("GetMixFormat returned no format");
        }
        // SAFETY: `pwfx` is a valid WAVEFORMATEX returned by GetMixFormat; read_unaligned copes with
        // the packed layout.
        let wf = unsafe { std::ptr::read_unaligned(pwfx) };
        let tag = wf.wFormatTag;
        let bits = wf.wBitsPerSample;
        let rate = wf.nSamplesPerSec;
        let channels = wf.nChannels;
        let block_align = usize::from(wf.nBlockAlign);
        let float = if tag == 0xFFFE {
            // SAFETY: a format tagged WAVE_FORMAT_EXTENSIBLE is a WAVEFORMATEXTENSIBLE in memory.
            let ext = unsafe { std::ptr::read_unaligned(pwfx.cast::<WAVEFORMATEXTENSIBLE>()) };
            let sub = ext.SubFormat;
            // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT starts with 0x00000003.
            sub.data1 == 3
        } else {
            tag == 3
        };
        let kind = match (float, bits) {
            (true, 32) => SampleKind::F32,
            (false, 16) => SampleKind::I16,
            (false, 24) => SampleKind::I24,
            (false, 32) => SampleKind::I32,
            _ => {
                // SAFETY: frees the format GetMixFormat allocated; not used afterwards.
                unsafe { CoTaskMemFree(Some(pwfx.cast_const().cast())) };
                bail!("unsupported capture format: tag {tag}, {bits} bits")
            }
        };
        // SAFETY: `client` is live and `pwfx` is its own valid mix format; 100 ms buffer, shared mode,
        // event callback.
        let init = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                EVENTCALLBACK,
                1_000_000,
                0,
                pwfx.cast_const(),
                None,
            )
        };
        // SAFETY: frees the format GetMixFormat allocated; Initialize has copied what it needs.
        unsafe { CoTaskMemFree(Some(pwfx.cast_const().cast())) };
        init.context("IAudioClient::Initialize")?;
        // SAFETY: creates an unnamed auto-reset event; no security attributes.
        let event =
            unsafe { CreateEventW(None, false, false, PCWSTR::null()) }.context("CreateEventW")?;
        // SAFETY: `client` is initialised for event callbacks; `event` is a live event handle.
        unsafe { client.SetEventHandle(event) }.context("SetEventHandle")?;
        // SAFETY: `client` is initialised.
        let capture: IAudioCaptureClient =
            unsafe { client.GetService() }.context("IAudioCaptureClient")?;
        // SAFETY: `client` is initialised and has its event handle.
        unsafe { client.Start() }.context("IAudioClient::Start")?;
        Ok(Self {
            _com: com,
            client,
            capture,
            event,
            device_name,
            sample_rate: rate,
            channels,
            kind,
            block_align,
        })
    }

    /// Waits up to `timeout_ms` for data, then returns the next packet, or None when there is none.
    pub fn next_packet(&mut self, timeout_ms: u32) -> Result<Option<Packet>> {
        use windows::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: `event` is a live event handle owned by self.
        let _ = unsafe { WaitForSingleObject(self.event, timeout_ms) };
        // SAFETY: `capture` is live.
        let pending = unsafe { self.capture.GetNextPacketSize() }.context("GetNextPacketSize")?;
        if pending == 0 {
            return Ok(None);
        }
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut frames = 0u32;
        let mut flags = 0u32;
        // SAFETY: the three out-pointers point to live locals; the position outputs are not wanted.
        unsafe {
            self.capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
        }
        .context("GetBuffer")?;
        // AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY = 1, AUDCLNT_BUFFERFLAGS_SILENT = 2.
        let discontinuity = flags & 1 != 0;
        let silent = flags & 2 != 0;
        let n = frames as usize * usize::from(self.channels);
        let mut samples = vec![0.0f32; n];
        if !silent && !data.is_null() {
            // SAFETY: GetBuffer returned `frames` frames of `block_align` bytes at `data`, valid until
            // ReleaseBuffer below; the bytes are copied out before then.
            let bytes =
                unsafe { std::slice::from_raw_parts(data, frames as usize * self.block_align) };
            convert_samples(self.kind, bytes, &mut samples);
        }
        // SAFETY: releases exactly the `frames` frames GetBuffer returned.
        unsafe { self.capture.ReleaseBuffer(frames) }.context("ReleaseBuffer")?;
        Ok(Some(Packet {
            samples,
            frames,
            discontinuity,
        }))
    }
}

impl Drop for WasapiCapture {
    fn drop(&mut self) {
        // SAFETY: `client` is live; stopping a stream that is already stopped is harmless.
        let _ = unsafe { self.client.Stop() };
        // SAFETY: `event` was created by CreateEventW in `open` and is closed once, here.
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.event) };
    }
}

/// Logical processors in this process's affinity mask (what a job limits it to).
pub fn affinity_count() -> Option<u32> {
    affinity_mask().map(usize::count_ones)
}

/// This process's affinity mask.
pub fn affinity_mask() -> Option<usize> {
    let mut process_mask = 0usize;
    let mut system_mask = 0usize;
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no cleanup.
    let process = unsafe { windows::Win32::System::Threading::GetCurrentProcess() };
    // SAFETY: `process` is valid and both out-pointers point to live usize values.
    unsafe {
        windows::Win32::System::Threading::GetProcessAffinityMask(
            process,
            &mut process_mask,
            &mut system_mask,
        )
    }
    .ok()?;
    Some(process_mask)
}

/// COM and Media Foundation for the calling thread.
struct MfSession;

impl MfSession {
    fn start() -> Result<Self> {
        // SAFETY: initialises COM for this thread; balanced by CoUninitialize in Drop.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .context("CoInitializeEx")?;
        // SAFETY: MFStartup with the SDK's own version constant; balanced by MFShutdown in Drop.
        if let Err(e) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) } {
            // SAFETY: balances the CoInitializeEx above on this thread.
            unsafe { CoUninitialize() };
            bail!("MFStartup: {e}");
        }
        Ok(Self)
    }
}

impl Drop for MfSession {
    fn drop(&mut self) {
        // SAFETY: balances the MFStartup in `start` on this thread.
        let _ = unsafe { MFShutdown() };
        // SAFETY: balances the CoInitializeEx in `start` on this thread.
        unsafe { CoUninitialize() };
    }
}

/// Decodes a whole audio file with Media Foundation. Returns the decoded duration in ms.
pub fn mf_decode_ms(path: &Path) -> Result<u64> {
    let _mf = MfSession::start()?;
    let url = HSTRING::from(path);
    // SAFETY: `url` is a valid NUL-terminated wide string for the call; no attributes.
    let reader: IMFSourceReader = unsafe { MFCreateSourceReaderFromURL(&url, None) }
        .context("MFCreateSourceReaderFromURL")?;
    let stream = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    let mut total_100ns: i64 = 0;
    loop {
        let mut flags = 0u32;
        let mut sample: Option<IMFSample> = None;
        // SAFETY: `reader` is live; the flags and sample out-pointers point to live locals.
        unsafe { reader.ReadSample(stream, 0, None, Some(&mut flags), None, Some(&mut sample)) }
            .context("ReadSample")?;
        if flags & (MF_SOURCE_READERF_ERROR.0 as u32) != 0 {
            bail!("decode error");
        }
        if let Some(s) = sample {
            // SAFETY: `s` is a live IMFSample the reader returned.
            total_100ns += unsafe { s.GetSampleDuration() }.unwrap_or(0);
        }
        if flags & (MF_SOURCE_READERF_ENDOFSTREAM.0 as u32) != 0 {
            break;
        }
    }
    Ok(u64::try_from(total_100ns / 10_000).unwrap_or(0))
}

#[cfg(test)]
mod wasapi_tests {
    use super::*;

    #[test]
    fn converts_16_bit_samples() {
        let bytes = [0x00, 0x40, 0x00, 0xC0]; // 16384 and -16384
        let mut out = [0.0f32; 2];
        convert_samples(SampleKind::I16, &bytes, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-6 && (out[1] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn converts_24_bit_samples() {
        let bytes = [0x00, 0x00, 0x40, 0x00, 0x00, 0xC0]; // +0.5 and -0.5
        let mut out = [0.0f32; 2];
        convert_samples(SampleKind::I24, &bytes, &mut out);
        assert!((out[0] - 0.5).abs() < 1e-6 && (out[1] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn converts_32_bit_int_and_float_samples() {
        let mut out = [0.0f32; 1];
        convert_samples(SampleKind::I32, &0x4000_0000i32.to_le_bytes(), &mut out);
        assert!((out[0] - 0.5).abs() < 1e-6);
        convert_samples(SampleKind::F32, &0.25f32.to_le_bytes(), &mut out);
        assert!((out[0] - 0.25).abs() < 1e-9);
    }
}
