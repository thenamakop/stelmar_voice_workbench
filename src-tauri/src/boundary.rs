//! The command boundary: every command runs its work on a blocking worker inside
//! `catch_unwind`, so a bug that panics in a command becomes an error the UI shows as a Toast and
//! never unwinds into the WebView2 or Tauri event loop. The recorder runs on its own thread and is
//! not affected either way (wb-recorder, recorder.rs).

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Text of a panic payload.
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

/// Runs `f` off the async runtime with panics caught.
pub async fn blocking<T, F>(name: &'static str, f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(move || catch_unwind(AssertUnwindSafe(f))).await {
        Ok(Ok(result)) => result,
        Ok(Err(payload)) => {
            let msg = panic_message(payload.as_ref());
            tracing::error!(
                command = name,
                "panic caught at the command boundary: {msg}"
            );
            Err(format!(
                "Internal error in {name}. Recording is not affected. Details are in the app log."
            ))
        }
        Err(e) => {
            tracing::error!(command = name, "command worker failed: {e}");
            Err(format!("{name} could not run: {e}"))
        }
    }
}

/// Logs every panic (with its thread) before unwinding continues.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let msg = panic_message(info.payload());
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        tracing::error!(thread = name, location = at, "panic: {msg}");
        default(info);
    }));
}
