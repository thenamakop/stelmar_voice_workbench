//! The speech engine's decisions that need no model, so they are tested in seconds without building
//! whisper.cpp: chunking at VAD pauses ([`chunker`]), the language of each chunk ([`lang`]),
//! output guards ([`guards`]) and Hindi/Urdu/English text handling ([`text`]).
//!
//! `wb-engine` (the process) wires these to whisper.cpp through whisper-rs.

pub mod chunker;
pub mod guards;
pub mod lang;
pub mod text;
