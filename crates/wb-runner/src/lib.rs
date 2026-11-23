//! Runs the post-processing queue (Phase 4 step 4): one session's jobs and units ([`session_queue`]), the VAD
//! pauses of each closed part from `wb-engine chunks` ([`chunks`]), the gate every engine process passes
//! ([`gate`]: the scheduler's pause rules and the 3 GB start reserve), running a unit through the supervisor
//! ([`exec`]) and the manager that ties them together ([`manager`]). No Tauri in here: the app supplies the
//! machine, power and recorder readings, the profile and the UI events.

pub mod chunks;
pub mod exec;
pub mod gate;
pub mod manager;
pub mod session_queue;
