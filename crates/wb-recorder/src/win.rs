//! Windows calls the recorder needs: MMCSS registration, thread priority and background mode,
//! keeping the machine awake, thread CPU time, free disk space and the performance counter.
//!
//! This is the only module in `wb-recorder` that may use `unsafe` (docs/UNSAFE.md). Each block
//! holds one call and says why it is sound. Callers use the safe functions in `crate::sys`.
#![allow(unsafe_code)]

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::Media::Audio::{
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioClient, IMMDeviceEnumerator,
    MMDeviceEnumerator,
};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_SYSTEM_REQUIRED, EXECUTION_STATE, SetThreadExecutionState,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, GetCurrentThread,
    GetThreadInformation, GetThreadTimes, MEMORY_PRIORITY_INFORMATION, SetThreadPriority,
    THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END, THREAD_PRIORITY,
    THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_HIGHEST, ThreadMemoryPriority,
};
use windows::core::HSTRING;

/// Registration of the current thread with the Multimedia Class Scheduler Service. Reverted on
/// drop; must be dropped on the thread that created it.
pub struct MmcssGuard(HANDLE);

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 is the handle AvSetMmThreadCharacteristicsW returned for this thread and
        // it is reverted exactly once, here.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.0) };
    }
}

/// Registers the current thread with MMCSS under `task` (for example "Pro Audio").
pub fn mmcss_register(task: &str) -> windows::core::Result<MmcssGuard> {
    let name = HSTRING::from(task);
    let mut index = 0u32;
    // SAFETY: `name` is a valid NUL-terminated wide string that outlives the call, and `index`
    // is a valid u32 the call may write (0 asks MMCSS to assign a task index).
    let handle = unsafe { AvSetMmThreadCharacteristicsW(&name, &mut index) }?;
    Ok(MmcssGuard(handle))
}

fn set_current_thread_priority(priority: THREAD_PRIORITY) -> windows::core::Result<()> {
    // SAFETY: GetCurrentThread returns a pseudo-handle that needs no cleanup and is always valid
    // on the calling thread.
    let thread = unsafe { GetCurrentThread() };
    // SAFETY: `thread` is the current thread's pseudo-handle and `priority` is a defined level.
    unsafe { SetThreadPriority(thread, priority) }
}

/// Raises the current thread to THREAD_PRIORITY_HIGHEST (the fallback when MMCSS refuses).
pub fn set_current_thread_highest() -> windows::core::Result<()> {
    set_current_thread_priority(THREAD_PRIORITY_HIGHEST)
}

/// Lowers the current thread to THREAD_PRIORITY_BELOW_NORMAL (mirror and hashing threads).
pub fn set_current_thread_below_normal() -> windows::core::Result<()> {
    set_current_thread_priority(THREAD_PRIORITY_BELOW_NORMAL)
}

/// Puts the current thread in background mode: lower CPU priority, lower I/O priority and lower
/// memory priority, so work such as hashing multi-GB model files does not compete with the
/// recorder for the disk. Ends with [`end_background_mode`] on the same thread. Fails with
/// ERROR_THREAD_MODE_ALREADY_BACKGROUND when the thread is in the mode already.
pub fn begin_background_mode() -> windows::core::Result<()> {
    set_current_thread_priority(THREAD_MODE_BACKGROUND_BEGIN)
}

/// Leaves background mode and restores the thread's earlier priorities.
pub fn end_background_mode() -> windows::core::Result<()> {
    set_current_thread_priority(THREAD_MODE_BACKGROUND_END)
}

/// The current thread's memory priority (1 very low to 5 normal). Background mode lowers it to
/// 1, which is how the mode is confirmed from outside.
pub fn current_memory_priority() -> Option<u32> {
    let mut info = MEMORY_PRIORITY_INFORMATION::default();
    // SAFETY: GetCurrentThread returns a pseudo-handle that needs no cleanup.
    let thread = unsafe { GetCurrentThread() };
    // SAFETY: `info` is a live MEMORY_PRIORITY_INFORMATION and the size passed is its size, as
    // the ThreadMemoryPriority class requires.
    unsafe {
        GetThreadInformation(
            thread,
            ThreadMemoryPriority,
            std::ptr::addr_of_mut!(info).cast(),
            std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
        )
    }
    .ok()?;
    Some(info.MemoryPriority.0)
}

/// Keeps the system out of sleep while the calling thread lives, or until `allow_sleep` is called
/// on the same thread. The display may still turn off.
pub fn keep_system_awake() -> bool {
    let flags = EXECUTION_STATE(ES_CONTINUOUS.0 | ES_SYSTEM_REQUIRED.0);
    // SAFETY: SetThreadExecutionState takes flags by value and touches no memory of ours.
    let previous = unsafe { SetThreadExecutionState(flags) };
    previous.0 != 0
}

/// Clears the calling thread's keep-awake request.
pub fn allow_sleep() {
    // SAFETY: as above; ES_CONTINUOUS alone clears the thread's earlier request.
    let _ = unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
}

fn filetime_100ns(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Kernel plus user CPU time used by the calling thread, in milliseconds.
pub fn current_thread_cpu_ms() -> Option<u64> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: GetCurrentThread returns a pseudo-handle that needs no cleanup.
    let thread = unsafe { GetCurrentThread() };
    // SAFETY: `thread` is valid on this thread and the four out-pointers point to live FILETIMEs.
    let ok = unsafe { GetThreadTimes(thread, &mut creation, &mut exit, &mut kernel, &mut user) };
    ok.ok()?;
    Some((filetime_100ns(kernel) + filetime_100ns(user)) / 10_000)
}

/// GetDriveTypeW of a drive root such as `D:\` (4 is DRIVE_REMOTE, a mapped network drive).
pub fn drive_type(root: &Path) -> u32 {
    let name = HSTRING::from(root);
    // SAFETY: `name` is a valid NUL-terminated wide string that outlives the call.
    unsafe { GetDriveTypeW(&name) }
}

/// Bytes free for the caller on the volume that holds `path`.
pub fn disk_free_bytes(path: &Path) -> windows::core::Result<u64> {
    let name = HSTRING::from(path);
    let mut free = 0u64;
    // SAFETY: `name` is a valid NUL-terminated wide string that outlives the call and `free` is a
    // valid u64 the call writes; the two optional totals are not requested.
    unsafe { GetDiskFreeSpaceExW(&name, Some(&mut free), None, None) }?;
    Ok(free)
}

/// `QueryPerformanceCounter` ticks (0 if the call fails).
pub fn qpc_ticks() -> u64 {
    let mut ticks = 0i64;
    // SAFETY: `ticks` is a valid i64 the call writes; the function takes no other input.
    let ok = unsafe { QueryPerformanceCounter(&mut ticks) };
    if ok.is_ok() { ticks.max(0) as u64 } else { 0 }
}

/// `QueryPerformanceFrequency`: ticks per second (0 if the call fails).
pub fn qpc_hz() -> u64 {
    let mut hz = 0i64;
    // SAFETY: `hz` is a valid i64 the call writes; the function takes no other input.
    let ok = unsafe { QueryPerformanceFrequency(&mut hz) };
    if ok.is_ok() { hz.max(0) as u64 } else { 0 }
}

/// What a shared-mode WASAPI capture client was given for a requested buffer duration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferGrant {
    /// Frames in the endpoint buffer (`IAudioClient::GetBufferSize`).
    pub frames: u32,
    /// Sample rate of the mix format the grant was read with.
    pub rate: u32,
    /// The engine's default period (`GetDevicePeriod`), in milliseconds.
    pub period_ms: f64,
}

/// Asks the audio engine what endpoint buffer it grants in shared mode for `requested_hns`
/// (100 ns units): opens a separate, short-lived `IAudioClient` on the endpoint with the same
/// event-driven shared-mode `Initialize` call cpal makes, reads `GetBufferSize` and
/// `GetDevicePeriod`, and releases it without starting it. cpal does not expose the granted size.
/// Runs on a helper thread with its own COM apartment, so it cannot disturb the caller's.
pub fn probe_shared_capture_buffer(endpoint_id: &str, requested_hns: i64) -> Option<BufferGrant> {
    // One probe at a time: a driver that hangs inside Initialize leaves its helper thread behind,
    // and a second one is not started on top of it.
    if PROBE_BUSY.swap(true, Ordering::AcqRel) {
        return None;
    }
    let id = endpoint_id.to_string();
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("wb-buffer-probe".into())
        .spawn(move || {
            // Released on every exit, a panic included.
            let _busy = ProbeSlot;
            let grant = probe_on_this_thread(&id, requested_hns);
            let _ = tx.send(grant);
        });
    if spawned.is_err() {
        PROBE_BUSY.store(false, Ordering::Release);
        return None;
    }
    // A hung driver must not hold the capture thread: give up after the timeout and carry on
    // without the granted size (the thread, if it ever returns, frees the slot itself).
    rx.recv_timeout(PROBE_TIMEOUT).ok().flatten()
}

/// Set while a probe helper thread runs.
static PROBE_BUSY: AtomicBool = AtomicBool::new(false);

/// Frees the probe slot when the helper thread ends, normally or by a panic.
struct ProbeSlot;

impl Drop for ProbeSlot {
    fn drop(&mut self) {
        PROBE_BUSY.store(false, Ordering::Release);
    }
}
/// How long the capture thread waits for the probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

fn probe_on_this_thread(endpoint_id: &str, requested_hns: i64) -> Option<BufferGrant> {
    // SAFETY: COM is initialised for this helper thread only and uninitialised before it ends;
    // every COM object is dropped (released) inside `probe`, before CoUninitialize runs.
    let init = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if init.is_err() {
        return None;
    }
    let out = probe(endpoint_id, requested_hns);
    // SAFETY: balances the successful CoInitializeEx above, on the same thread.
    unsafe { CoUninitialize() };
    out
}

fn probe(endpoint_id: &str, requested_hns: i64) -> Option<BufferGrant> {
    let id = HSTRING::from(endpoint_id);
    // SAFETY: COM is initialised on this thread; the class id and flags are the documented ones
    // for the device enumerator and no outer unknown is aggregated.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    // SAFETY: `id` is a NUL-terminated wide string that outlives the call.
    let device = unsafe { enumerator.GetDevice(&id) }.ok()?;
    // SAFETY: no activation parameters are passed; the requested interface is IAudioClient.
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }.ok()?;
    // SAFETY: the call returns a CoTaskMem allocation that is freed below.
    let format = unsafe { client.GetMixFormat() }.ok()?;
    // SAFETY: `format` is the pointer GetMixFormat just returned and is still allocated, so
    // reading its sample rate is valid.
    let rate = unsafe { (*format).nSamplesPerSec };
    // SAFETY: `format` is valid for the call (see above); an event-driven shared-mode client is
    // initialised exactly as cpal does, and is never started.
    let init = unsafe {
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            requested_hns,
            0,
            format,
            None,
        )
    };
    // SAFETY: `format` came from GetMixFormat (a CoTaskMem allocation), is freed exactly once and
    // is not used afterwards.
    unsafe { CoTaskMemFree(Some(format.cast_const().cast())) };
    init.ok()?;
    // SAFETY: the client was initialised successfully above.
    let frames = unsafe { client.GetBufferSize() }.ok()?;
    let mut period = 0i64;
    // SAFETY: `period` is a valid i64 the call writes; the minimum period is not requested.
    unsafe { client.GetDevicePeriod(Some(&mut period), None) }.ok()?;
    Some(BufferGrant {
        frames,
        rate,
        period_ms: period as f64 / 10_000.0,
    })
}
