//! Engine protocol shared by the app and `wb-engine`.
//!
//! Engines speak versioned JSON lines on stdout and log on stderr. Phase 0 only fixes the
//! protocol version and the envelope; the `info`, `bench`, `batch` and `live` messages arrive
//! in Phase 2.

use serde::{Deserialize, Serialize};

/// Protocol version. Bump on any incompatible change to a message.
pub const PROTOCOL_VERSION: u32 = 0;

/// The first line every engine prints on stdout, so the app can reject a mismatched engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// Always [`PROTOCOL_VERSION`] of the engine that wrote it.
    pub protocol: u32,
    /// Engine crate version, for logs.
    pub engine_version: String,
}

impl Hello {
    /// The hello line for an engine built from this crate version.
    pub fn new(engine_version: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            engine_version: engine_version.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trips_as_json() -> Result<(), serde_json::Error> {
        let hello = Hello::new("0.1.0");
        let line = serde_json::to_string(&hello)?;
        assert_eq!(line, r#"{"protocol":0,"engine_version":"0.1.0"}"#);
        let back: Hello = serde_json::from_str(&line)?;
        assert_eq!(back, hello);
        Ok(())
    }
}
