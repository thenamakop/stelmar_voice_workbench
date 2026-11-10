//! The engine supervisor: starts one or more `wb-engine` processes at the right priority in a
//! kill-on-close Job Object, reads their events, commits their segments in audio order (the single
//! writer), watches them and restarts them from the last committed segment after a crash, a kill or
//! 60 seconds of silence. No engine failure reaches the recorder: this crate shares no lock and no
//! thread with it (PLAN section 7).

pub mod commit;
pub mod process;
pub mod run;
#[cfg(windows)]
mod win;

pub use commit::Committer;
pub use process::{EngineProcess, Line, Priority, SpawnSpec, sidecar_path};
pub use run::{Notice, Observer, Policy, RunError, RunReport, run_batch};
