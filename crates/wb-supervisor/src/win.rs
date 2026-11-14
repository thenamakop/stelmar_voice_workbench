//! The Windows Job Object that holds an engine process, and starting an engine suspended.
//!
//! The only module in `wb-supervisor` that may use `unsafe` (docs/UNSAFE.md). Each engine gets a job of its
//! own, created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when the app exits or crashes, Windows ends every
//! engine (AGENTS.md: engine processes sit in a Job Object), and an optional per-process commit limit
//! (`JOB_OBJECT_LIMIT_PROCESS_MEMORY`) belongs to one engine, whose measured peak decides it. Inside an
//! emulation job (`wb-emulate`) these jobs are nested in it, which Windows 8 and later allow. An engine is
//! created suspended, put in its job and only then resumed, so no instruction of it runs outside the job.
#![allow(unsafe_code)]

use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
};
use windows::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
use windows::core::PCWSTR;

/// A job handle. Closing it kills every process still in it.
pub struct Job(HANDLE);

// SAFETY: a job handle is just an identifier the kernel validates; it may be used from any thread.
unsafe impl Send for Job {}
// SAFETY: as above; the only operation, AssignProcessToJobObject, is thread-safe in the kernel.
unsafe impl Sync for Job {}

impl Job {
    /// A job whose processes may each commit at most `process_memory_bytes` (when given).
    pub fn with_memory_limit(process_memory_bytes: Option<u64>) -> Result<Self, String> {
        // SAFETY: no security attributes and no name: an anonymous job owned by this process.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|e| format!("CreateJobObjectW: {e}"))?;
        let job = Self(handle);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(limit) = process_memory_bytes {
            info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
            info.ProcessMemoryLimit = usize::try_from(limit).unwrap_or(usize::MAX);
        }
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        // SAFETY: `info` is a fully initialised JOBOBJECT_EXTENDED_LIMIT_INFORMATION of `size` bytes
        // that lives for the call; the class matches the structure.
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                size,
            )
        }
        .map_err(|e| format!("SetInformationJobObject: {e}"))?;
        Ok(job)
    }

    /// Puts a started child in the job.
    pub fn assign(&self, child: &Child) -> Result<(), String> {
        let process = HANDLE(child.as_raw_handle());
        // SAFETY: both handles are valid: the job is ours and the child handle is held by `child`
        // for the length of the call.
        unsafe { AssignProcessToJobObject(self.0, process) }
            .map_err(|e| format!("AssignProcessToJobObject: {e}"))
    }
}

impl Job {
    /// The largest commit any process of this job has reached (it stays readable after the process ends).
    pub fn peak_process_memory(&self) -> Option<u64> {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        // SAFETY: `info` is a writable JOBOBJECT_EXTENDED_LIMIT_INFORMATION of `size` bytes that lives for the
        // call and the class matches the structure; the job handle is ours.
        unsafe {
            QueryInformationJobObject(
                Some(self.0),
                JobObjectExtendedLimitInformation,
                std::ptr::from_mut(&mut info).cast(),
                size,
                None,
            )
        }
        .ok()?;
        u64::try_from(info.PeakProcessMemoryUsed).ok()
    }
}

/// Resumes every thread of a process that was created suspended (a process made with `CREATE_SUSPENDED` has
/// one, its main thread). Returns how many threads were resumed.
pub fn resume_process(pid: u32) -> Result<u32, String> {
    // SAFETY: TH32CS_SNAPTHREAD with process id 0 snapshots every thread on the system; no pointer is passed.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
        .map_err(|e| format!("CreateToolhelp32Snapshot: {e}"))?;
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut resumed = 0u32;
    // SAFETY: `snapshot` is the valid handle returned above and `entry` is initialised with its size.
    let mut more = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
    while more {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: opens a thread by id with the one right needed; the result is a handle we close below.
            let opened = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) };
            if let Ok(thread) = opened {
                // SAFETY: `thread` is a valid handle with THREAD_SUSPEND_RESUME access.
                let previous = unsafe { ResumeThread(thread) };
                // SAFETY: `thread` came from OpenThread and is closed exactly once.
                let _ = unsafe { CloseHandle(thread) };
                if previous != u32::MAX {
                    resumed += 1;
                }
            }
        }
        // SAFETY: as for Thread32First.
        more = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: `snapshot` came from CreateToolhelp32Snapshot and is closed exactly once.
    let _ = unsafe { CloseHandle(snapshot) };
    if resumed == 0 {
        Err(format!("no thread of process {pid} could be resumed"))
    } else {
        Ok(resumed)
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: self.0 is the handle CreateJobObjectW returned; closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
