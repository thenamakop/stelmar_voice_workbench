//! Protocol output: one JSON line per event on stdout, flushed at once. Logs go to stderr only, so
//! the app can read stdout as a clean event stream (`crates/wb-protocol`).

use std::io::Write;

use wb_protocol::Event;

/// Writes `event` as one line to stdout. A broken pipe (the app went away) is an error the caller
/// should treat as "stop now": there is nobody to report to.
pub fn emit(event: &Event) -> std::io::Result<()> {
    let line = event.to_line().map_err(std::io::Error::other)?;
    let mut out = std::io::stdout().lock();
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}

/// A log line on stderr (never on stdout).
pub fn log(message: impl AsRef<str>) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "wb-engine: {}", message.as_ref());
}

/// A non-fatal problem the app should show: stderr and a `warning` event.
pub fn warn(code: &str, message: impl Into<String>) {
    let message = message.into();
    log(format!("warning {code}: {message}"));
    let _ = emit(&Event::Warning {
        code: code.into(),
        message,
    });
}
