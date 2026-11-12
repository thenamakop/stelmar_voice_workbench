//! What this process may really use (affinity mask, job memory limit), read once at start.
//!
//! Inside an emulation job `std::thread::available_parallelism()` and the default async runtime
//! see the host's 32 processors; the runtime here is sized from the affinity mask instead, and
//! the engine supervisor takes thread counts and memory budgets from the same view
//! (`docs/GATES.md`, Phase 1 -> 2).

use std::sync::OnceLock;

use wb_machine::Machine;

static MACHINE: OnceLock<Machine> = OnceLock::new();
/// The runtime must outlive the app: Tauri keeps only a handle to it.
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The machine view, read on first use.
pub fn get() -> &'static Machine {
    MACHINE.get_or_init(wb_machine::probe)
}

/// Replaces Tauri's default async runtime (one worker per processor the host reports) by one with
/// one worker per logical processor in the affinity mask. Returns the worker count; keeps the
/// default runtime and returns `None` if the new one cannot be built. Call before anything uses
/// `tauri::async_runtime`.
pub fn install_runtime() -> Option<usize> {
    let workers = get().logical_processors.max(1);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .thread_name("wb-async")
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("could not build the async runtime with {workers} workers: {err}");
            return None;
        }
    };
    let handle = RUNTIME.get_or_init(|| runtime).handle().clone();
    tauri::async_runtime::set(handle);
    Some(workers)
}
