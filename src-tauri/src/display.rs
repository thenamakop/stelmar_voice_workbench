//! Display guard: recover from WebView2 process failures without touching the recording
//! (Phase 0 open item: an overlay hook crashed the WebView2 browser process and left a white
//! window). A renderer failure reloads the page; a browser-process failure replaces the window;
//! GPU-process failures recover inside WebView2. Every failure is logged; repeated failures show a
//! warning. The recorder runs in Rust threads and never depends on the webview.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::state::{AppState, lock};

/// This many failures within `WINDOW` show a warning.
const REPEATED: usize = 3;
const WINDOW: Duration = Duration::from_secs(600);

static NEXT_WINDOW: AtomicU32 = AtomicU32::new(2);
static RECENT: Mutex<VecDeque<Instant>> = Mutex::new(VecDeque::new());

/// Watches `window`'s webview for process failures.
pub fn guard(window: &WebviewWindow) {
    #[cfg(windows)]
    {
        let app = window.app_handle().clone();
        let label = window.label().to_string();
        let result = window.with_webview(move |webview| {
            let controller = webview.controller();
            let app_cb = app.clone();
            let label_cb = label.clone();
            let registered = crate::win::on_process_failed(&controller, move |kind, sender| {
                failed(&app_cb, &label_cb, kind, sender);
            });
            match registered {
                Ok(()) => tracing::info!(window = label, "display guard installed"),
                Err(e) => tracing::error!(window = label, "display guard not installed: {e}"),
            }
        });
        if let Err(e) = result {
            tracing::error!("display guard: could not reach the webview: {e}");
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

#[cfg(windows)]
fn failed(
    app: &AppHandle,
    label: &str,
    kind: crate::win::FailureKind,
    sender: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2>,
) {
    use crate::win::FailureKind;
    let recording = crate::recording::is_recording(app);
    tracing::error!(window = label, ?kind, recording, "WebView2 process failed");
    let count = note_failure(app);
    match kind {
        FailureKind::Browser => {
            // Replace the window from outside this callback, after it returns.
            let app = app.clone();
            let old = label.to_string();
            let _ = std::thread::Builder::new()
                .name("wb-display-recover".into())
                .spawn(move || {
                    std::thread::sleep(Duration::from_millis(250));
                    let app2 = app.clone();
                    if let Err(e) = app.run_on_main_thread(move || recreate(&app2, &old)) {
                        tracing::error!("display recovery could not reach the main thread: {e}");
                    }
                });
        }
        FailureKind::Render | FailureKind::RenderUnresponsive | FailureKind::FrameRender => {
            match sender {
                Some(webview) => match crate::win::reload(&webview) {
                    Ok(()) => {
                        tracing::warn!(window = label, "display reloaded after a renderer failure")
                    }
                    Err(e) => tracing::error!(window = label, "display reload failed: {e}"),
                },
                None => tracing::error!(
                    window = label,
                    "renderer failed but no webview was given to reload"
                ),
            }
        }
        FailureKind::Gpu | FailureKind::Other(_) => {
            tracing::warn!(window = label, "WebView2 restarts this process by itself");
        }
    }
    let st = app.state::<AppState>();
    st.broadcast(serde_json::json!({ "kind": "display_recovered", "restarts": count, "failure": format!("{kind:?}") }));
}

/// Counts a failure; sets the snapshot warning when failures repeat. Returns the total so far.
fn note_failure(app: &AppHandle) -> u32 {
    let now = Instant::now();
    let repeated = {
        let mut recent = lock(&RECENT);
        recent.push_back(now);
        while recent
            .front()
            .is_some_and(|t| now.duration_since(*t) > WINDOW)
        {
            recent.pop_front();
        }
        recent.len() >= REPEATED
    };
    let st = app.state::<AppState>();
    let mut snap = lock(&st.snapshot);
    snap.display_restarts += 1;
    if repeated {
        snap.display_warning = Some(format!(
            "The display restarted {} times in 10 minutes. Recording was not affected. If it keeps happening, close overlay and monitoring tools (RivaTuner, MSI Afterburner) and restart the app after this session.",
            REPEATED
        ));
    }
    snap.display_restarts
}

/// Opens a new main window from the configuration, guards it, then closes the dead one. The new
/// window is created first, so the app never has zero windows and does not exit.
fn recreate(app: &AppHandle, old_label: &str) {
    let Some(mut config) = app.config().app.windows.first().cloned() else {
        tracing::error!("display recovery: no window configuration");
        return;
    };
    let n = NEXT_WINDOW.fetch_add(1, Ordering::Relaxed);
    config.label = format!("main-{n}");
    match tauri::WebviewWindowBuilder::from_config(app, &config)
        .and_then(tauri::WebviewWindowBuilder::build)
    {
        Ok(window) => {
            guard(&window);
            tracing::warn!(
                new = config.label,
                old = old_label,
                "display recovered in a new window"
            );
            if let Some(old) = app.get_webview_window(old_label)
                && let Err(e) = old.destroy()
            {
                tracing::error!("could not close the failed window: {e}");
            }
        }
        Err(e) => tracing::error!("display recovery: could not open a new window: {e}"),
    }
}
