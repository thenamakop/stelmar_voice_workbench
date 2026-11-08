//! `wb-engine`: the speech engine process. Phase 0 only prints its version.
//!
//! `wb-engine` (no arguments) or `wb-engine --version` prints `wb-engine <version>`.
//! `wb-engine hello` prints the protocol hello line as JSON.

use std::io::Write;
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);
    let line = match arg.as_deref() {
        None | Some("--version") | Some("-V") => format!("wb-engine {VERSION}"),
        Some("hello") => match serde_json::to_string(&wb_protocol::Hello::new(VERSION)) {
            Ok(json) => json,
            Err(err) => {
                eprintln!("wb-engine: could not encode hello: {err}");
                return ExitCode::FAILURE;
            }
        },
        Some(other) => {
            eprintln!(
                "wb-engine: unknown command '{other}' (Phase 0 supports --version and hello)"
            );
            return ExitCode::from(2);
        }
    };
    let mut out = std::io::stdout().lock();
    if writeln!(out, "{line}").is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
