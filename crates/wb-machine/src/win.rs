//! Windows calls that tell a process what it may really use: its affinity mask, the processor
//! cores, host memory, and the Job Object it runs in (the emulation job under `wb-emulate`).
//!
//! The only module in `wb-machine` that may use `unsafe` (docs/UNSAFE.md). Each block holds one
//! call and says why it is sound; parsing of what comes back happens in safe code (`lib.rs`).
#![allow(unsafe_code)]

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    JOB_OBJECT_LIMIT_JOB_MEMORY, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList,
    JobObjectExtendedLimitInformation, QueryInformationJobObject,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformationEx, GlobalMemoryStatusEx, MEMORYSTATUSEX, RelationProcessorCore,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessAffinityMask, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// This process's affinity mask (processor group 0).
pub fn process_affinity() -> Option<u64> {
    let mut process_mask = 0usize;
    let mut system_mask = 0usize;
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no cleanup.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `process` is valid and both out-pointers point to live usize values.
    unsafe { GetProcessAffinityMask(process, &mut process_mask, &mut system_mask) }.ok()?;
    Some(process_mask as u64)
}

/// Raw `RelationProcessorCore` records from GetLogicalProcessorInformationEx.
pub fn processor_core_records() -> Option<Vec<u8>> {
    let mut len = 0u32;
    // SAFETY: a null buffer with a length out-pointer asks for the size needed; the call fails
    // with ERROR_INSUFFICIENT_BUFFER, which is expected here.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len) };
    if len == 0 {
        return None;
    }
    // u64 storage keeps the records 8-byte aligned, as the structures require.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let ptr = buf
        .as_mut_ptr()
        .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
    // SAFETY: `buf` holds at least `len` writable, aligned bytes, and `len` says so.
    unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, Some(ptr), &mut len) }.ok()?;
    Some(
        buf.iter()
            .flat_map(|w| w.to_le_bytes())
            .take(len as usize)
            .collect(),
    )
}

/// Physical memory of the host: (total, available) in bytes.
pub fn host_memory() -> Option<(u64, u64)> {
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: `status` is a live MEMORYSTATUSEX with dwLength set, as the call requires.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some((status.ullTotalPhys, status.ullAvailPhys))
}

/// The commit limit of the Job Object this process is in, if it has one. A null job handle asks
/// about the calling process's own job. Under nested jobs this is the innermost one.
pub fn job_memory_limit() -> Option<u64> {
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    // SAFETY: `info` is a live JOBOBJECT_EXTENDED_LIMIT_INFORMATION of `size` bytes, matching the
    // class; a None handle means "the job of the calling process".
    unsafe {
        QueryInformationJobObject(
            None,
            JobObjectExtendedLimitInformation,
            std::ptr::from_mut(&mut info).cast(),
            size,
            None,
        )
    }
    .ok()?;
    let limited = info.BasicLimitInformation.LimitFlags.0 & JOB_OBJECT_LIMIT_JOB_MEMORY.0 != 0;
    limited.then_some(info.JobMemoryLimit as u64)
}

/// Process ids in the calling process's job (empty when it is in none).
fn job_process_ids() -> Vec<u32> {
    const MAX: usize = 512;
    // The list structure is followed by room for MAX ids; u64 storage keeps it aligned.
    let bytes =
        std::mem::size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() + MAX * std::mem::size_of::<usize>();
    let mut buf = vec![0u64; bytes.div_ceil(8)];
    // SAFETY: `buf` is zeroed, aligned and at least `bytes` long, which the query is told.
    let ok = unsafe {
        QueryInformationJobObject(
            None,
            JobObjectBasicProcessIdList,
            buf.as_mut_ptr().cast(),
            (buf.len() * 8) as u32,
            None,
        )
    };
    if ok.is_err() {
        return Vec::new();
    }
    // Layout (x64): NumberOfAssignedProcesses u32 @0, NumberOfProcessIdsInList u32 @4, then ids
    // as usize @8.
    let raw: Vec<u8> = buf.iter().flat_map(|w| w.to_le_bytes()).collect();
    let in_list = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]) as usize;
    (0..in_list.min(MAX))
        .filter_map(|i| {
            let o = 8 + i * 8;
            raw.get(o..o + 8).map(|b| {
                u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as u32
            })
        })
        .collect()
}

/// Commit charge of every process in the calling process's job, in bytes (None when it is in no
/// job or nothing could be read).
pub fn job_commit_used() -> Option<u64> {
    let ids = job_process_ids();
    if ids.is_empty() {
        return None;
    }
    let mut total = 0u64;
    let mut read_any = false;
    for pid in ids {
        // SAFETY: OpenProcess takes the id by value; a failure is an Err, handled below.
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) })
        else {
            continue;
        };
        let guard = Handle(handle);
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        // SAFETY: `guard.0` is an open process handle and `counters` is a live structure whose
        // size is given.
        let ok = unsafe { GetProcessMemoryInfo(guard.0, &mut counters, counters.cb) };
        if ok.is_ok() {
            total += counters.PagefileUsage as u64;
            read_any = true;
        }
    }
    read_any.then_some(total)
}

struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: self.0 is the handle OpenProcess returned; closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
