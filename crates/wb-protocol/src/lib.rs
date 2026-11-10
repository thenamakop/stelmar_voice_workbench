//! Engine protocol shared by the app and `wb-engine`.
//!
//! An engine is a separate process (`wb-engine <info|bench|batch|live> [--request FILE]`). The
//! app writes a [`Request`] as JSON to a file and passes its path; the engine answers with JSON
//! lines on stdout, one [`Event`] per line, and logs on stderr. `live` also reads 16 kHz mono f32
//! little-endian PCM on stdin. The first line of stdout is always [`Event::Hello`], so the app can
//! reject an engine of another protocol version before it reads anything else.
//!
//! Times are milliseconds on the audio clock (from the start of the session), never wall-clock
//! (AGENTS.md). Every segment carries `lang`, `lang_prob` and `mixed`.
//!
//! Exit codes: 0 done, 2 bad usage or request, 3 the CPU lacks features this build needs, 4 a
//! model or the VAD model could not be loaded, 5 the device could not be used, 1 any other error.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Protocol version. Bump on any incompatible change to a message.
pub const PROTOCOL_VERSION: u32 = 1;

pub mod exit {
    pub const OK: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const USAGE: u8 = 2;
    pub const CPU_FEATURES: u8 = 3;
    pub const MODEL: u8 = 4;
    pub const DEVICE: u8 = 5;
}

// ------------------------------------------------------------------------------------ common

/// A model file the engine may load. The app has already verified `sha256` against the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRef {
    /// Manifest id, for example `medium` or `hindi-medium-q8_0`.
    pub id: String,
    /// Absolute path of the ggml file.
    pub path: String,
}

/// Where the engine runs inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Device {
    Cpu,
    /// The GPU with this index in the engine's own (de-duplicated) device list from `info`.
    Gpu {
        index: u32,
    },
}

/// Decoding settings of a role (PLAN section 6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// `near-live`, `rolling`, `night`, `live` or `eval`; for the logs.
    #[serde(default)]
    pub role: String,
    /// 1 is greedy.
    pub beam_size: i32,
    pub best_of: i32,
    pub temperature_fallback: bool,
    /// Carry the last `context_chars` characters of committed text in the same language.
    pub previous_text_context: bool,
    pub context_chars: usize,
    /// Segment times come from the audio clock and the VAD, never from model timestamps.
    pub timestamps: bool,
    /// Glossary prompt, in Latin script.
    #[serde(default)]
    pub glossary: String,
    /// Longest chunk handed to the model, in seconds. Without model timestamps whisper.cpp stops at
    /// its token limit on a 30 s window, so chunks stay at or under 15 s.
    pub max_chunk_s: f32,
    /// Close chunks at sentence-sized pauses (sessions with more than one language). `false` merges
    /// speech greedily up to `max_chunk_s`, as the evaluation harness does.
    #[serde(default = "yes")]
    pub sentence_split: bool,
    /// Shortest chunk worth detecting a language for, in seconds.
    pub min_chunk_s: f32,
    /// Silero VAD speech-probability threshold and the shortest pause that ends a chunk.
    pub vad_threshold: f32,
    pub vad_min_silence_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            role: "rolling".into(),
            beam_size: 5,
            best_of: 5,
            temperature_fallback: true,
            previous_text_context: true,
            context_chars: 200,
            timestamps: false,
            glossary: String::new(),
            max_chunk_s: 15.0,
            sentence_split: true,
            min_chunk_s: 2.0,
            vad_threshold: 0.5,
            vad_min_silence_ms: 100,
        }
    }
}

/// One entry of the operator's language log: from `audio_ms` on, detection is locked to `lang`
/// (`None` returns to detection among the session's languages).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LangLogEntry {
    pub audio_ms: u64,
    pub lang: Option<String>,
}

// ----------------------------------------------------------------------------------- requests

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Info(InfoRequest),
    Bench(BenchRequest),
    Batch(BatchRequest),
    Live(LiveRequest),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfoRequest {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchRequest {
    pub model: ModelRef,
    pub vad_model: ModelRef,
    pub lang: String,
    /// 16 kHz mono WAV, or any WAV the engine can resample.
    pub clip: String,
    pub mode: BenchMode,
    pub device: Device,
    pub threads: u32,
    /// Runs after the warm-up; the median is reported.
    pub runs: u32,
    /// If the warm-up run is slower than this real-time factor, make one timed run instead of `runs`
    /// (the speed tables mark such a cell "impractical: single run").
    #[serde(default)]
    pub single_run_above_rtf: Option<f64>,
    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchMode {
    /// Beam 5, temperature fallback, 15 s chunks (rolling and night passes).
    Rolling,
    /// Greedy on utterance-length chunks (near-live).
    NearLive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A session folder: the engine reads `parts.json` and the part files.
    Session { dir: String },
    /// One WAV file (evaluation, bench).
    File { path: String },
}

/// A work unit: audio from `start_ms` to `end_ms`, cut at VAD pauses by the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    pub index: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchRequest {
    pub source: Source,
    /// Empty means the whole source as one unit.
    #[serde(default)]
    pub units: Vec<Unit>,
    /// Language code to the model that decodes it; `"*"` is the model for any other language.
    pub models_by_lang: BTreeMap<String, ModelRef>,
    pub vad_model: ModelRef,
    /// The session's expected languages; detection is restricted to them.
    pub languages: Vec<String>,
    #[serde(default)]
    pub lang_log: Vec<LangLogEntry>,
    #[serde(default)]
    pub settings: Settings,
    pub device: Device,
    pub threads: u32,
    /// Chunks that end at or before this audio time are skipped (a restart after a kill resumes
    /// from the last committed segment; nothing is repeated).
    #[serde(default)]
    pub resume_from_ms: u64,
    /// Language detection per chunk. `false` decodes everything in `languages[0]` (a session with
    /// one language, or the evaluation harness with the language given).
    #[serde(default = "yes")]
    pub detect_language: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveRequest {
    pub models_by_lang: BTreeMap<String, ModelRef>,
    pub vad_model: ModelRef,
    pub languages: Vec<String>,
    #[serde(default)]
    pub settings: Settings,
    pub device: Device,
    pub threads: u32,
}

// ------------------------------------------------------------------------------------ events

/// Which whisper.cpp an engine runs: the same release, commit and source tree on every build, so
/// the evidence of any run shows what produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhisperCppInfo {
    /// `whisper_version()` of the linked library.
    pub version: String,
    /// The pinned git commit (`third_party/README.md`).
    pub commit: String,
    /// SHA-256 of the vendored source tree, computed by `build.rs` at build time.
    pub tree_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    /// Always [`PROTOCOL_VERSION`] of the engine that wrote it.
    pub protocol: u32,
    /// Engine crate version, for logs.
    pub engine_version: String,
    /// `avx2`, `avx512`, `vulkan` or `cuda` (the build variant), or `dev` for a plain cargo build.
    #[serde(default)]
    pub variant: String,
    #[serde(default)]
    pub whisper_cpp: Option<WhisperCppInfo>,
}

impl Hello {
    /// The hello line for an engine built from this crate version.
    pub fn new(engine_version: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            engine_version: engine_version.into(),
            variant: String::new(),
            whisper_cpp: None,
        }
    }
}

/// A language detection result for a chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LangProb {
    pub lang: String,
    pub prob: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentFlag {
    /// Dropped by a guard (PLAN section 6) and flagged for review; never shown as a caption.
    Repetition,
    /// A known hallucination on silence ("Thanks for watching" and similar).
    SilencePhrase,
    NoSpeech,
    CompressionRatio,
    WrongScript,
    /// The chunk reached the model's token limit; text may be cut.
    Truncated,
    /// Incomplete UTF-8 at the end of the model output was dropped.
    BrokenUtf8,
}

/// One decoded segment, in audio order within its work unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentEvent {
    pub unit: u32,
    /// Index of the VAD chunk in the source, counted from 0.
    pub chunk: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    pub lang: String,
    pub lang_prob: f32,
    /// More than one script in the text (Devanagari with Latin words, for example).
    pub mixed: bool,
    /// `ltr`, `rtl` or `auto`, for rendering.
    pub dir: String,
    pub avg_logprob: f32,
    pub no_speech_prob: f32,
    pub compression_ratio: f32,
    /// Model id that produced the text.
    pub model: String,
    /// The chunk was decoded in both candidate languages and this was the better one.
    #[serde(default)]
    pub decoded_both_ways: bool,
    /// Detection scores among the session's languages, best first.
    #[serde(default)]
    pub detection: Vec<LangProb>,
    #[serde(default)]
    pub flags: Vec<SegmentFlag>,
    /// Whisper's token count for the chunk, when it reached the limit.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackendDevice {
    /// Index in this list, the one `Device::Gpu` refers to.
    pub index: u32,
    /// ggml backend: `CPU`, `Vulkan` or `CUDA`.
    pub backend: String,
    pub name: String,
    pub description: String,
    /// `cpu`, `gpu` or `igpu` (integrated, shares RAM with the CPU).
    pub kind: String,
    pub total_memory_bytes: u64,
    pub free_memory_bytes: u64,
    /// Adapter driver version, when the OS reports it.
    #[serde(default)]
    pub driver_version: Option<String>,
    /// Entries of the raw list that described this same physical adapter and were dropped.
    #[serde(default)]
    pub duplicates_dropped: u32,
    /// For an integrated GPU: the system memory it can use (it shares RAM with the CPU).
    #[serde(default)]
    pub shared_memory_bytes: Option<u64>,
    /// The adapter's name as Windows (DXGI) reports it, when it could be matched.
    #[serde(default)]
    pub adapter_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuReport {
    pub brand: String,
    pub logical_processors: u32,
    pub physical_cores: u32,
    /// Instruction sets the CPU has (AVX, AVX2, FMA, F16C, AVX-512 parts).
    pub features: Vec<String>,
    /// What this build needs; `info` exits with code 3 when one is missing.
    pub required_features: Vec<String>,
    pub missing_features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfoReport {
    pub engine_version: String,
    pub variant: String,
    pub whisper_cpp: WhisperCppInfo,
    /// whisper.cpp's own system-info string (shows the instruction sets it was compiled with).
    pub system_info: String,
    pub cpu: CpuReport,
    /// ggml backends registered in this build.
    pub backends: Vec<String>,
    pub devices: Vec<BackendDevice>,
    /// Raw device list entries before de-duplication.
    pub raw_device_count: u32,
    /// Machine view from the affinity mask and the job limit.
    pub logical_processors_allowed: u32,
    pub available_ram_bytes: u64,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    pub model: String,
    pub variant: String,
    pub whisper_cpp: WhisperCppInfo,
    pub device: Device,
    pub device_name: String,
    pub threads: u32,
    pub mode: BenchMode,
    pub clip_seconds: f64,
    pub lang: String,
    pub load_s: f64,
    /// Processing time of each timed run divided by the clip length.
    pub rtf_runs: Vec<f64>,
    pub rtf_median: f64,
    pub warmup_rtf: f64,
    /// True when the warm-up showed an RTF over the cap and only one timed run was made.
    pub single_run: bool,
    pub peak_working_set_bytes: u64,
    pub peak_commit_bytes: u64,
    /// GPU memory in use by this process after the runs, where the OS reports it.
    pub gpu_local_bytes: Option<u64>,
    pub gpu_shared_bytes: Option<u64>,
    pub segments: u32,
    pub truncated_chunks: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub unit: u32,
    pub chunks: u32,
    pub audio_ms: u64,
    pub decode_s: f64,
    pub rtf: f64,
    pub peak_working_set_bytes: u64,
    pub detections: u32,
    pub both_ways: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoneSummary {
    pub segments: u32,
    pub chunks: u32,
    pub audio_ms: u64,
    pub truncated_chunks: u32,
    pub elapsed_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Hello(Hello),
    /// Models and the VAD are loaded; the engine starts work.
    Ready {
        device: Device,
        device_name: String,
        threads: u32,
        models: Vec<String>,
        load_s: f64,
    },
    Progress {
        unit: u32,
        /// Audio time up to which every chunk of this unit is decoded.
        committed_until_ms: u64,
        unit_end_ms: u64,
    },
    Segment(SegmentEvent),
    Stats(Stats),
    Info(Box<InfoReport>),
    Bench(Box<BenchReport>),
    /// Voice activity from `live` (decoding arrives in Phase 7).
    Vad {
        start_ms: u64,
        end_ms: u64,
    },
    Warning {
        code: String,
        message: String,
    },
    Error {
        code: String,
        message: String,
        fatal: bool,
    },
    Done(DoneSummary),
}

impl Event {
    /// One JSON line, without the trailing newline.
    pub fn to_line(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }
}

// -------------------------------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(value: &T) -> Result<T, serde_json::Error>
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        serde_json::from_str(&serde_json::to_string(value)?)
    }

    fn model(id: &str) -> ModelRef {
        ModelRef {
            id: id.into(),
            path: format!("models/{id}.bin"),
        }
    }

    #[test]
    fn hello_is_the_first_line_and_names_the_version() -> Result<(), serde_json::Error> {
        let line = Event::Hello(Hello::new("0.1.0")).to_line()?;
        assert!(
            line.starts_with(r#"{"event":"hello","protocol":1,"engine_version":"0.1.0""#),
            "{line}"
        );
        match Event::from_line(&line)? {
            Event::Hello(h) => assert_eq!(h.protocol, PROTOCOL_VERSION),
            other => panic!("not a hello: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn a_hello_from_an_older_engine_without_the_new_fields_still_reads()
    -> Result<(), serde_json::Error> {
        let h: Hello = serde_json::from_str(r#"{"protocol":0,"engine_version":"0.1.0"}"#)?;
        assert_eq!(h.protocol, 0);
        assert!(h.whisper_cpp.is_none());
        Ok(())
    }

    #[test]
    fn requests_round_trip() -> Result<(), serde_json::Error> {
        let batch = Request::Batch(BatchRequest {
            source: Source::Session {
                dir: "D:/S/day1/hall-a".into(),
            },
            units: vec![Unit {
                index: 0,
                start_ms: 0,
                end_ms: 300_000,
            }],
            models_by_lang: BTreeMap::from([
                ("en".into(), model("medium")),
                ("hi".into(), model("hindi-medium-q8_0")),
            ]),
            vad_model: model("silero-v6.2.0"),
            languages: vec!["en".into(), "hi".into()],
            lang_log: vec![
                LangLogEntry {
                    audio_ms: 60_000,
                    lang: Some("hi".into()),
                },
                LangLogEntry {
                    audio_ms: 120_000,
                    lang: None,
                },
            ],
            settings: Settings::default(),
            device: Device::Gpu { index: 1 },
            threads: 3,
            resume_from_ms: 42_000,
            detect_language: true,
        });
        assert_eq!(round_trip(&batch)?, batch);
        for r in [
            Request::Info(InfoRequest {}),
            Request::Live(LiveRequest {
                models_by_lang: BTreeMap::from([("*".into(), model("small"))]),
                vad_model: model("silero-v6.2.0"),
                languages: vec!["en".into()],
                settings: Settings::default(),
                device: Device::Cpu,
                threads: 2,
            }),
            Request::Bench(BenchRequest {
                model: model("medium"),
                vad_model: model("silero-v6.2.0"),
                lang: "hi".into(),
                clip: "clip.wav".into(),
                mode: BenchMode::Rolling,
                device: Device::Cpu,
                threads: 4,
                runs: 3,
                single_run_above_rtf: Some(2.0),
                settings: Settings::default(),
            }),
        ] {
            assert_eq!(round_trip(&r)?, r);
        }
        Ok(())
    }

    #[test]
    fn a_batch_request_needs_only_the_essentials() -> Result<(), serde_json::Error> {
        let text = r#"{"type":"batch","source":{"kind":"file","path":"a.wav"},
            "models_by_lang":{"*":{"id":"medium","path":"m.bin"}},
            "vad_model":{"id":"silero-v6.2.0","path":"v.bin"},
            "languages":["hi"],"device":{"kind":"cpu"},"threads":8}"#;
        match serde_json::from_str::<Request>(text)? {
            Request::Batch(b) => {
                assert!(b.units.is_empty() && b.lang_log.is_empty());
                assert_eq!(b.resume_from_ms, 0);
                assert!(b.detect_language);
                assert_eq!(b.settings.max_chunk_s, 15.0);
                assert!(!b.settings.timestamps);
            }
            other => panic!("not a batch: {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn a_segment_keeps_devanagari_and_urdu_text_exactly() -> Result<(), serde_json::Error> {
        for text in [
            "नमस्ते, यह एक test है।",
            "یہ ایک ٹیسٹ ہے۔ STELMAR 2027",
            "mixed हिंदी and English",
        ] {
            let seg = Event::Segment(SegmentEvent {
                unit: 1,
                chunk: 7,
                start_ms: 1_000,
                end_ms: 4_500,
                text: text.into(),
                lang: "hi".into(),
                lang_prob: 0.93,
                mixed: true,
                dir: "auto".into(),
                avg_logprob: -0.31,
                no_speech_prob: 0.02,
                compression_ratio: 1.4,
                model: "hindi-medium-q8_0".into(),
                decoded_both_ways: false,
                detection: vec![
                    LangProb {
                        lang: "hi".into(),
                        prob: 0.93,
                    },
                    LangProb {
                        lang: "en".into(),
                        prob: 0.06,
                    },
                ],
                flags: vec![SegmentFlag::Truncated],
                truncated: true,
            });
            let line = seg.to_line()?;
            assert!(!line.contains('\n'));
            assert_eq!(Event::from_line(&line)?, seg);
        }
        Ok(())
    }

    #[test]
    fn every_event_is_one_json_line() -> Result<(), serde_json::Error> {
        let events = vec![
            Event::Ready {
                device: Device::Cpu,
                device_name: "CPU".into(),
                threads: 4,
                models: vec!["medium".into()],
                load_s: 1.5,
            },
            Event::Progress {
                unit: 0,
                committed_until_ms: 5_000,
                unit_end_ms: 300_000,
            },
            Event::Vad {
                start_ms: 0,
                end_ms: 800,
            },
            Event::Warning {
                code: "truncated".into(),
                message: "chunk 3 reached the token limit".into(),
            },
            Event::Error {
                code: "model".into(),
                message: "x\ny".into(),
                fatal: true,
            },
            Event::Done(DoneSummary {
                segments: 3,
                chunks: 3,
                audio_ms: 20_000,
                truncated_chunks: 0,
                elapsed_s: 4.0,
            }),
        ];
        for e in events {
            let line = e.to_line()?;
            assert!(!line.contains('\n'), "{line}");
            assert_eq!(Event::from_line(&line)?, e);
        }
        Ok(())
    }

    #[test]
    fn the_device_names_are_stable() -> Result<(), serde_json::Error> {
        assert_eq!(serde_json::to_string(&Device::Cpu)?, r#"{"kind":"cpu"}"#);
        assert_eq!(
            serde_json::to_string(&Device::Gpu { index: 2 })?,
            r#"{"kind":"gpu","index":2}"#
        );
        Ok(())
    }
}
