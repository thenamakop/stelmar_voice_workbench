//! The Windows Job Object that holds every engine process.
//!
//! The only module in `wb-supervisor` that may use `unsafe` (docs/UNSAFE.md). One job for the app,
//! created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`: when the app exits or crashes, Windows ends every
//! engine in it (AGENTS.md: engine processes sit in a Job Object). Inside an emulation job (`wb-emulate`)
//! this job is nested in it, which Windows 8 and later allow.
#![allow(unsafe_code)]

use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::core::PCWSTR;

/// A job handle. Closing it kills every process still in it.
pub struct Job(HANDLE);

// SAFETY: a job handle is just an identifier the kernel validates; it may be used from any thread.
unsafe impl Send for Job {}
// SAFETY: as above; the only operation, AssignProcessToJobObject, is thread-safe in the kernel.
unsafe impl Sync for Job {}

impl Job {
    pub fn new() -> Result<Self, String> {
        // SAFETY: no security attributes and no name: an anonymous job owned by this process.
        let handle = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|e| format!("CreateJobObjectW: {e}"))?;
        let job = Self(handle);
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
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

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: self.0 is the handle CreateJobObjectW returned; closed exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
