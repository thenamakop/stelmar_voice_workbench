//! Native calls the engine needs: the ggml backend device list, DXGI adapters and process memory.
//!
//! The only module in `wb-engine` that may use `unsafe` (docs/UNSAFE.md). Each block holds one
//! call and says why it is sound. What comes back is turned into plain Rust values here, and the
//! decisions (which devices are the same adapter, which are integrated) are made in safe code in
//! `devices.rs`.
#![allow(unsafe_code)]

use std::ffi::CStr;

use whisper_rs::whisper_rs_sys as sys;

/// What ggml says about one device of the build's backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDevice {
    /// Position in ggml's own list (CPU, GPUs and accelerators together).
    pub raw_index: usize,
    pub name: String,
    pub description: String,
    pub kind: RawKind,
    pub free: u64,
    pub total: u64,
    /// ggml's id for the adapter when the backend knows one (a PCI bus id for Vulkan).
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    Cpu,
    Gpu,
    /// Integrated: shares memory with the CPU.
    Igpu,
    Accel,
    Other,
}

fn text(p: *const std::os::raw::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: ggml returns NUL-terminated strings that live as long as the device registry (the
    // whole process); the pointer was just checked for null.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Every device ggml registered for this build, in ggml's order.
pub fn ggml_devices() -> Vec<RawDevice> {
    // SAFETY: no arguments; initialises the static backend registry on first use.
    let count = unsafe { sys::ggml_backend_dev_count() };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        // SAFETY: `i` is below the count just read.
        let dev = unsafe { sys::ggml_backend_dev_get(i) };
        if dev.is_null() {
            continue;
        }
        // SAFETY: an all-zero ggml_backend_dev_props is a valid value (null pointers, zero numbers,
        // false flags); the call below fills it.
        let mut props: sys::ggml_backend_dev_props = unsafe { std::mem::zeroed() };
        // SAFETY: `dev` is a valid device and `props` a live structure of the right type.
        unsafe { sys::ggml_backend_dev_get_props(dev, &mut props) };
        let kind = match props.type_ {
            sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_CPU => RawKind::Cpu,
            sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU => RawKind::Gpu,
            sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_IGPU => RawKind::Igpu,
            sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_ACCEL => RawKind::Accel,
            _ => RawKind::Other,
        };
        let device_id = text(props.device_id);
        out.push(RawDevice {
            raw_index: i,
            name: text(props.name),
            description: text(props.description),
            kind,
            free: props.memory_free as u64,
            total: props.memory_total as u64,
            device_id: (!device_id.is_empty()).then_some(device_id),
        });
    }
    out
}

/// Names of the ggml backends compiled into this build (`CPU`, `Vulkan`, `CUDA`).
pub fn ggml_backends() -> Vec<String> {
    // SAFETY: no arguments; initialises the static backend registry on first use.
    let count = unsafe { sys::ggml_backend_reg_count() };
    (0..count)
        .map(|i| {
            // SAFETY: `i` is below the count just read.
            let reg = unsafe { sys::ggml_backend_reg_get(i) };
            if reg.is_null() {
                return String::new();
            }
            // SAFETY: `reg` is a valid registry entry.
            text(unsafe { sys::ggml_backend_reg_name(reg) })
        })
        .filter(|n| !n.is_empty())
        .collect()
}

// ------------------------------------------------------------------------------- Windows only

/// A display adapter as DXGI describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub dedicated_video_memory: u64,
    pub shared_system_memory: u64,
    /// `A.B.C.D` from `CheckInterfaceSupport` (the user-mode driver version), when available.
    pub driver_version: Option<String>,
    pub software: bool,
}

#[cfg(windows)]
pub fn adapters() -> Vec<Adapter> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1,
    };
    use windows::core::Interface;

    // SAFETY: creates a factory; the result is checked.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut i = 0u32;
    loop {
        // SAFETY: `i` counts up from zero; the call fails with DXGI_ERROR_NOT_FOUND past the end.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        i += 1;
        let adapter: IDXGIAdapter1 = adapter;
        // SAFETY: `adapter` is a valid adapter.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let end = desc
            .Description
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..end]);
        // SAFETY: asks whether the adapter supports the IDXGIDevice interface; the returned
        // version number is a plain i64.
        let driver_version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }
            .ok()
            .map(|v| {
                format!(
                    "{}.{}.{}.{}",
                    (v >> 48) & 0xFFFF,
                    (v >> 32) & 0xFFFF,
                    (v >> 16) & 0xFFFF,
                    v & 0xFFFF
                )
            });
        out.push(Adapter {
            name,
            vendor_id: desc.VendorId,
            device_id: desc.DeviceId,
            dedicated_video_memory: desc.DedicatedVideoMemory as u64,
            shared_system_memory: desc.SharedSystemMemory as u64,
            driver_version,
            software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
        });
    }
    out
}

#[cfg(not(windows))]
pub fn adapters() -> Vec<Adapter> {
    Vec::new()
}

/// Peak working set and peak commit of this process so far, in bytes.
#[cfg(windows)]
pub fn peak_memory() -> (u64, u64) {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut c = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no cleanup.
    let me = unsafe { GetCurrentProcess() };
    // SAFETY: `me` is valid and `c` a live structure whose size is given.
    if unsafe { GetProcessMemoryInfo(me, &mut c, c.cb) }.is_ok() {
        (c.PeakWorkingSetSize as u64, c.PeakPagefileUsage as u64)
    } else {
        (0, 0)
    }
}

#[cfg(not(windows))]
pub fn peak_memory() -> (u64, u64) {
    (0, 0)
}

/// Video memory this process uses on the adapter called `adapter_name`: (local, non-local) in
/// bytes. Local is the adapter's own memory; non-local is system memory it uses (all of an
/// integrated GPU's memory, shared with the CPU). None if the adapter is not found.
#[cfg(windows)]
pub fn gpu_memory_in_use(adapter_name: &str) -> Option<(u64, u64)> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL,
        DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter3, IDXGIFactory1,
    };
    use windows::core::Interface;

    // SAFETY: creates a factory; the result is checked.
    let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }.ok()?;
    let mut i = 0u32;
    // SAFETY: `i` counts up from zero; the call fails past the last adapter.
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
        i += 1;
        // SAFETY: `adapter` is a valid adapter.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let end = desc
            .Description
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(desc.Description.len());
        if String::from_utf16_lossy(&desc.Description[..end]) != adapter_name {
            continue;
        }
        let Ok(a3) = adapter.cast::<IDXGIAdapter3>() else {
            continue;
        };
        let query = |group| -> u64 {
            let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
            // SAFETY: node 0, a valid segment group and a live out-structure.
            if unsafe { a3.QueryVideoMemoryInfo(0, group, &mut info) }.is_ok() {
                info.CurrentUsage
            } else {
                0
            }
        };
        return Some((
            query(DXGI_MEMORY_SEGMENT_GROUP_LOCAL),
            query(DXGI_MEMORY_SEGMENT_GROUP_NON_LOCAL),
        ));
    }
    None
}

#[cfg(not(windows))]
pub fn gpu_memory_in_use(_adapter_name: &str) -> Option<(u64, u64)> {
    None
}
