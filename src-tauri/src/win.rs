//! WebView2 calls for the display guard: register a ProcessFailed handler and reload a webview.
//!
//! The only module in the app crate that may use `unsafe` (docs/UNSAFE.md). Each block holds one
//! call and says why it is sound.
#![allow(unsafe_code)]

use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE, ICoreWebView2,
    ICoreWebView2Controller,
};
use webview2_com::ProcessFailedEventHandler;

/// Which WebView2 process failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The browser process ended: the webview is dead and must be recreated.
    Browser,
    /// The page's renderer ended or hung: a reload brings it back.
    Render,
    RenderUnresponsive,
    FrameRender,
    /// WebView2 restarts the GPU process by itself.
    Gpu,
    Other(i32),
}

fn kind_of(k: COREWEBVIEW2_PROCESS_FAILED_KIND) -> FailureKind {
    match k {
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => FailureKind::Browser,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => FailureKind::Render,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
            FailureKind::RenderUnresponsive
        }
        COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED => FailureKind::FrameRender,
        COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => FailureKind::Gpu,
        other => FailureKind::Other(other.0),
    }
}

/// Calls `on_failure` (on the UI thread) whenever a process of this webview fails. The webview
/// that raised it is passed so a renderer failure can be answered with `reload`.
pub fn on_process_failed(
    controller: &ICoreWebView2Controller,
    on_failure: impl Fn(FailureKind, Option<ICoreWebView2>) + 'static,
) -> windows::core::Result<()> {
    // SAFETY: `controller` is the live controller Tauri handed us on its UI thread.
    let core = unsafe { controller.CoreWebView2() }?;
    let handler = ProcessFailedEventHandler::create(Box::new(move |sender, args| {
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        if let Some(args) = args {
            // SAFETY: `args` is the live event-args object for this event; `kind` is a valid
            // out-pointer for the call.
            unsafe { args.ProcessFailedKind(&mut kind) }?;
        }
        on_failure(kind_of(kind), sender);
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: `core` is live, `handler` is a valid COM event handler WebView2 keeps a reference
    // to, and `token` is a valid out-pointer. The registration lasts as long as the webview.
    unsafe { core.add_ProcessFailed(&handler, &mut token) }?;
    Ok(())
}

/// Reloads the page in `webview` (after a renderer failure).
pub fn reload(webview: &ICoreWebView2) -> windows::core::Result<()> {
    // SAFETY: `webview` is the live ICoreWebView2 that raised the event, used on its UI thread.
    unsafe { webview.Reload() }
}
