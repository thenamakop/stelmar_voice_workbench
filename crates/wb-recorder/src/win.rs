//! Windows calls the recorder needs: MMCSS registration, thread priority and background mode,
//! keeping the machine awake, thread CPU time, free disk space and the performance counter.
//!
//! This is the only module in `wb-recorder` that may use `unsafe` (docs/UNSAFE.md). Each block
//! holds one call and says why it is sound. Callers use the safe functions in `crate::sys`.
#![allow(unsafe_code)]

use std::path::Path;

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW};
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
