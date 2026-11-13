//! Windows calls of the system check: power source, the names of running programs and of the
//! modules loaded into this process, and the product name from the registry.
//!
//! One of the modules that may use `unsafe` (docs/UNSAFE.md). Each block holds one call and says why
//! it is sound. Callers use the safe functions.
#![allow(unsafe_code)]

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
    Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{HSTRING, PCWSTR};

/// A snapshot handle closed on drop.
struct Snapshot(HANDLE);

impl Drop for Snapshot {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateToolhelp32Snapshot and is closed exactly once, here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

/// `Some(true)` on mains power, `Some(false)` on battery, `None` when Windows cannot say.
pub fn on_mains_power() -> Option<bool> {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: `status` is a valid SYSTEM_POWER_STATUS the call may write.
    unsafe { GetSystemPowerStatus(&mut status) }.ok()?;
    match status.ACLineStatus {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// File names of the running processes (for example `RTSS.exe`), with their ids.
pub fn running_processes() -> Vec<(u32, String)> {
    // SAFETY: the call takes no pointers; failure is returned as an error.
    let Ok(handle) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };
    let snap = Snapshot(handle);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut out = Vec::new();
    // SAFETY: `snap.0` is a live snapshot and `entry.dwSize` is set, as the call requires.
    let mut ok = unsafe { Process32FirstW(snap.0, &mut entry) }.is_ok();
    while ok {
        out.push((entry.th32ProcessID, wide_to_string(&entry.szExeFile)));
        // SAFETY: as above; the entry keeps its size between calls.
        ok = unsafe { Process32NextW(snap.0, &mut entry) }.is_ok();
    }
    out
}

/// File names of the modules loaded into process `pid` (0 is this process). Empty when the process
/// cannot be opened.
pub fn loaded_modules(pid: u32) -> Vec<String> {
    // SAFETY: the call takes no pointers; failure is returned as an error.
    let Ok(handle) =
        (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) })
    else {
        return Vec::new();
    };
    let snap = Snapshot(handle);
    let mut entry = MODULEENTRY32W {
        dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    let mut out = Vec::new();
    // SAFETY: `snap.0` is a live snapshot and `entry.dwSize` is set, as the call requires.
    let mut ok = unsafe { Module32FirstW(snap.0, &mut entry) }.is_ok();
    while ok {
        out.push(wide_to_string(&entry.szModule));
        // SAFETY: as above; the entry keeps its size between calls.
        ok = unsafe { Module32NextW(snap.0, &mut entry) }.is_ok();
    }
    out
}

/// A text value under `HKEY_LOCAL_MACHINE`, or `None` when it is missing.
pub fn registry_text(subkey: &str, value: &str) -> Option<String> {
    let key = HSTRING::from(subkey);
    let name = HSTRING::from(value);
    let mut buf = vec![0u16; 256];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: `key` and `name` are NUL-terminated wide strings that outlive the call; `buf` holds
    // `bytes` bytes and the call writes at most that many and updates `bytes`.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(name.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status.is_err() {
        return None;
    }
    let chars = (bytes as usize / 2).min(buf.len());
    Some(wide_to_string(&buf[..chars]))
}
