//! Safe wrappers over the platform calls in `win.rs`, with neutral fallbacks elsewhere so the
//! crate's logic still builds and tests on other systems.

use std::path::Path;

/// How the recorder thread's priority was raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriorityMode {
    /// Registered with MMCSS as "Pro Audio".
    Mmcss,
    /// MMCSS refused; THREAD_PRIORITY_HIGHEST instead.
    Highest,
    /// Neither worked; normal priority.
    Normal,
}

/// Keeps the recorder thread's priority raised while alive.
pub struct PriorityGuard {
    pub mode: PriorityMode,
    /// Why MMCSS was not used, when it was not.
    pub fallback_reason: Option<String>,
    #[cfg(windows)]
    _mmcss: Option<crate::win::MmcssGuard>,
}

/// Registers the calling thread with MMCSS "Pro Audio", falling back to THREAD_PRIORITY_HIGHEST.
pub fn raise_recorder_priority() -> PriorityGuard {
    #[cfg(windows)]
    {
        match crate::win::mmcss_register("Pro Audio") {
            Ok(guard) => PriorityGuard {
                mode: PriorityMode::Mmcss,
                fallback_reason: None,
                _mmcss: Some(guard),
            },
            Err(mmcss_err) => match crate::win::set_current_thread_highest() {
                Ok(()) => PriorityGuard {
                    mode: PriorityMode::Highest,
                    fallback_reason: Some(format!("MMCSS registration failed: {mmcss_err}")),
                    _mmcss: None,
                },
                Err(prio_err) => PriorityGuard {
                    mode: PriorityMode::Normal,
                    fallback_reason: Some(format!(
                        "MMCSS registration failed: {mmcss_err}; THREAD_PRIORITY_HIGHEST failed: {prio_err}"
                    )),
                    _mmcss: None,
                },
            },
        }
    }
    #[cfg(not(windows))]
    {
        PriorityGuard {
            mode: PriorityMode::Normal,
            fallback_reason: Some("not Windows".into()),
        }
    }
}

/// Lowers the calling thread to below-normal priority (mirror, hashing). Best effort.
pub fn lower_current_thread() {
    #[cfg(windows)]
    {
        let _ = crate::win::set_current_thread_below_normal();
    }
}

/// What a thread's background mode looks like from inside (for the log and for tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundState {
    /// Memory priority of the calling thread: 5 is normal, 1 is what background mode sets.
    /// `None` where it cannot be read.
    pub memory_priority: Option<u32>,
}

impl BackgroundState {
    /// True when the thread is in background mode (memory priority 1).
    pub fn is_background(&self) -> bool {
        self.memory_priority == Some(1)
    }
}

/// The calling thread's background state now.
pub fn background_state() -> BackgroundState {
    #[cfg(windows)]
    {
        BackgroundState {
            memory_priority: crate::win::current_memory_priority(),
        }
    }
    #[cfg(not(windows))]
    {
        BackgroundState {
            memory_priority: None,
        }
    }
}

/// Keeps the calling thread in Windows background mode (CPU, I/O and memory priority all low)
/// until dropped. For work that must never compete with the recorder, such as hashing the model
/// files at start-up. Create it on the thread that does the work and drop it on the same thread.
pub struct BackgroundGuard {
    /// True when this guard started the mode (and so ends it).
    active: bool,
    /// Why the mode could not be entered, when it could not.
    pub failure: Option<String>,
}

impl BackgroundGuard {
    /// Enters background mode. When the thread is in it already, the guard does not end it.
    /// A failure is kept in `failure`; the thread then runs at its normal priority.
    pub fn enter() -> Self {
        #[cfg(windows)]
        {
            match crate::win::begin_background_mode() {
                Ok(()) => Self {
                    active: true,
                    failure: None,
                },
                Err(e) => Self {
                    active: false,
                    failure: Some(e.to_string()),
                },
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                active: false,
                failure: Some("not Windows".into()),
            }
        }
    }
}

impl Drop for BackgroundGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.active {
            let _ = crate::win::end_background_mode();
        }
    }
}

/// Keeps the machine awake while the calling thread holds it; released on drop.
pub struct AwakeGuard {
    pub active: bool,
}

impl AwakeGuard {
    pub fn acquire() -> Self {
        #[cfg(windows)]
        {
            Self {
                active: crate::win::keep_system_awake(),
            }
        }
        #[cfg(not(windows))]
        {
            Self { active: false }
        }
    }
}

impl Drop for AwakeGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.active {
            crate::win::allow_sleep();
        }
    }
}

/// CPU time used by the calling thread so far, in milliseconds.
pub fn thread_cpu_ms() -> Option<u64> {
    #[cfg(windows)]
    {
        crate::win::current_thread_cpu_ms()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Bytes free for this user on the volume holding `path` (the nearest existing ancestor).
pub fn disk_free_bytes(path: &Path) -> std::io::Result<u64> {
    let existing = path.ancestors().find(|p| p.exists()).unwrap_or(path);
    #[cfg(windows)]
    {
        crate::win::disk_free_bytes(existing).map_err(|e| std::io::Error::other(e.to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = existing;
        Ok(u64::MAX)
    }
}

/// GetDriveTypeW's value for a mapped network drive.
#[cfg(windows)]
const DRIVE_REMOTE: u32 = 4;

/// True when `path` is on a drive in this computer (internal or USB): not a UNC share, not a
/// device path, not a mapped network drive, and no existing folder on the way is a symlink or
/// junction to one. Recordings and backups must never go over the network (AGENTS.md
/// non-negotiable 1). Links are read without following them, so the check itself never touches
/// the network.
pub fn is_local_path(path: &Path) -> bool {
    is_local_path_depth(path, 0)
}

/// `is_local_path`, following link targets with the same check (a chain of links), at most 8
/// deep. Each link is checked before anything below it is looked at, so a link to a share is
/// refused before the share is touched.
fn is_local_path_depth(path: &Path, depth: u8) -> bool {
    if depth > 8 || !is_local_prefix(path) {
        return false;
    }
    let mut cur = std::path::PathBuf::new();
    for c in path.components() {
        cur.push(c);
        let Ok(meta) = std::fs::symlink_metadata(&cur) else {
            break; // the rest does not exist yet; it will be created under a local folder
        };
        if meta.file_type().is_symlink() {
            let target = match std::fs::read_link(&cur) {
                Ok(t) if t.is_relative() => cur.parent().map_or(t.clone(), |p| p.join(&t)),
                Ok(t) => t,
                Err(_) => return false,
            };
            if !is_local_path_depth(&target, depth + 1) {
                return false;
            }
        }
    }
    true
}

fn is_local_prefix(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    match path.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                #[cfg(windows)]
                {
                    let root = format!("{}:{}", char::from(letter), std::path::MAIN_SEPARATOR);
                    crate::win::drive_type(Path::new(&root)) != DRIVE_REMOTE
                }
                #[cfg(not(windows))]
                {
                    let _ = letter;
                    true
                }
            }
            // A local volume mounted in a folder reads back as \\?\Volume{GUID}\ (only local
            // volumes can be mounted this way).
            Prefix::Verbatim(name) => name.to_string_lossy().starts_with("Volume{"),
            _ => false,
        },
        _ => !cfg!(windows),
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Runs `f` on a thread of its own, so a priority mode never leaks into the test runner's
    /// threads.
    fn on_new_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        match std::thread::spawn(f).join() {
            Ok(v) => v,
            Err(p) => std::panic::resume_unwind(p),
        }
    }

    #[test]
    fn background_mode_is_set_while_the_guard_lives_and_ended_after() {
        let (before, inside, after) = on_new_thread(|| {
            let before = background_state();
            let guard = BackgroundGuard::enter();
            assert_eq!(guard.failure, None);
            let inside = background_state();
            drop(guard);
            (before, inside, background_state())
        });
        assert!(!before.is_background(), "{before:?}");
        assert!(inside.is_background(), "{inside:?}");
        assert!(!after.is_background(), "{after:?}");
        assert_eq!(before.memory_priority, after.memory_priority);
    }

    #[test]
    fn entering_background_mode_twice_is_safe_and_the_inner_guard_does_not_end_it() {
        on_new_thread(|| {
            let outer = BackgroundGuard::enter();
            assert_eq!(outer.failure, None);
            let inner = BackgroundGuard::enter();
            assert!(inner.failure.is_some(), "the second enter is refused");
            drop(inner);
            assert!(
                background_state().is_background(),
                "still background after the inner guard"
            );
            drop(outer);
            assert!(!background_state().is_background());
        });
    }

    #[test]
    fn background_mode_belongs_to_the_thread_that_entered_it() {
        let _guard_thread = std::thread::spawn(|| {
            let _g = BackgroundGuard::enter();
            std::thread::sleep(std::time::Duration::from_millis(200));
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        on_new_thread(|| assert!(!background_state().is_background()));
    }

    #[test]
    fn network_paths_are_not_local() {
        assert!(!is_local_path(Path::new(r"\\srv\share\x")));
        assert!(!is_local_path(Path::new(r"\\?\UNC\srv\share\x")));
        assert!(!is_local_path(Path::new(r"\\.\pipe\x")));
        assert!(!is_local_path(Path::new("relative")));
        assert!(is_local_path(&std::env::temp_dir()));
        assert!(is_local_prefix(Path::new(
            r"\\?\Volume{0a1b2c3d-0000-0000-0000-000000000000}\rec"
        )));
    }

    /// a -> b (local) and b -> \\srv\share: the chain is followed and refused without touching
    /// the share. Skipped where creating symlinks is not allowed (no Developer Mode or admin).
    #[test]
    fn a_chain_of_links_to_a_share_is_not_local() -> std::io::Result<()> {
        let dir = crate::testutil::TempDir::new("sys-link-chain")?;
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        if std::os::windows::fs::symlink_dir(r"\\srv-that-does-not-exist\share", &b).is_err() {
            return Ok(());
        }
        std::os::windows::fs::symlink_dir(&b, &a)?;
        assert!(!is_local_path(&a.join("rec")));
        assert!(!is_local_path(&b));
        Ok(())
    }
}

/// `QueryPerformanceCounter` ticks now (0 where unavailable). Cheap, and safe to call from the
/// driver's error callback: it does not allocate or lock.
pub fn qpc_ticks() -> u64 {
    #[cfg(windows)]
    {
        crate::win::qpc_ticks()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Ticks per second of [`qpc_ticks`] (0 where unavailable).
pub fn qpc_hz() -> u64 {
    #[cfg(windows)]
    {
        crate::win::qpc_hz()
    }
    #[cfg(not(windows))]
    {
        0
    }
}
