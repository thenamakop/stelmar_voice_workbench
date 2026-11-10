//! `wb-engine`: the speech engine process (docs/prompts/phase-2-engines-and-models.md).
//!
//! ```text
//! wb-engine --version | hello
//! wb-engine info
//! wb-engine bench --request FILE
//! wb-engine batch --request FILE
//! wb-engine batch --file WAV --models en=medium,hi=hindi-medium --langs en,hi --out FILE.jsonl
//!                 [--settings JSON] [--threads N] [--device cpu|gpu:N] [--models-dir DIR]
//! wb-engine chunks --file WAV --out CHUNKS.json [--max-chunk-s 15] [--greedy] [--models-dir DIR]
//!                 (the VAD chunks batch would decode, without any model: for labelling)
//! wb-engine live  --request FILE        (16 kHz mono f32 PCM on stdin)
//! ```
//!
//! JSON lines on stdout (the first is always `hello`), logs on stderr. Exit codes are in
//! `wb_protocol::exit`: 3 means the CPU lacks an instruction set this build needs; it is checked
//! before anything touches whisper.cpp.

mod asr;
mod audio;
mod batch;
mod bench;
mod cpu;
mod devices;
mod info;
mod live;
mod out;
mod source;
mod win;

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use wb_protocol::{
    BatchRequest, BenchRequest, Device, Event, Hello, LiveRequest, ModelRef, Request, Settings,
    Source, exit,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A failure with the exit code the app should see.
struct Fail {
    code: u8,
    error_code: &'static str,
    message: String,
}

impl Fail {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: exit::USAGE,
            error_code: "usage",
            message: message.into(),
        }
    }

    fn from_error(err: &anyhow::Error) -> Self {
        let text = format!("{err:#}");
        if err.downcast_ref::<asr::LoadError>().is_some() {
            Self {
                code: exit::MODEL,
                error_code: "model",
                message: text,
            }
        } else if err.downcast_ref::<DeviceError>().is_some() {
            Self {
                code: exit::DEVICE,
                error_code: "device",
                message: text,
            }
        } else {
            Self {
                code: exit::ERROR,
                error_code: "error",
                message: text,
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct DeviceError(String);

fn version_line() -> String {
    let p = info::whisper_cpp_pinned();
    format!(
        "wb-engine {VERSION} ({}, whisper.cpp {} commit {} tree {})",
        info::variant(),
        p.version,
        &p.commit[..p.commit.len().min(12)],
        &p.tree_sha256[..p.tree_sha256.len().min(12)]
    )
}

fn hello() -> Event {
    Event::Hello(Hello {
        protocol: wb_protocol::PROTOCOL_VERSION,
        engine_version: VERSION.to_string(),
        variant: info::variant().to_string(),
        whisper_cpp: Some(info::whisper_cpp_pinned()),
    })
}

/// Prints `hello`, then refuses to go on when the CPU lacks what this build needs.
fn begin() -> Result<(), Fail> {
    out::emit(&hello()).map_err(|e| Fail {
        code: exit::ERROR,
        error_code: "io",
        message: e.to_string(),
    })?;
    let cpu = info::cpu_report();
    if !cpu.missing_features.is_empty() {
        let message = format!(
            "This engine build ({}) needs CPU features this computer does not have: {}. Use the avx2 build, or run the system check, which picks a build that fits.",
            info::variant(),
            cpu.missing_features.join(", ")
        );
        return Err(Fail {
            code: exit::CPU_FEATURES,
            error_code: "cpu_features",
            message,
        });
    }
    Ok(())
}

fn args_value<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn read_request(args: &[String]) -> Result<Request, Fail> {
    let path =
        args_value(args, "--request").ok_or_else(|| Fail::usage("--request FILE is required"))?;
    let text = std::fs::read_to_string(path)
        .map_err(|e| Fail::usage(format!("cannot read the request file {path}: {e}")))?;
    serde_json::from_str(&text).map_err(|e| {
        Fail::usage(format!(
            "the request file {path} is not a valid request: {e}"
        ))
    })
}

// ----------------------------------------------------------------------------- device picking

struct Picked {
    gpu_ordinal: Option<u32>,
    name: String,
    adapter: Option<String>,
}

fn pick_device(device: Device) -> Result<Picked, anyhow::Error> {
    match device {
        Device::Cpu => Ok(Picked {
            gpu_ordinal: None,
            name: "CPU".into(),
            adapter: None,
        }),
        Device::Gpu { index } => {
            let raw = win::ggml_devices();
            let kept = devices::merge(&raw, &win::adapters());
            let k = kept
                .iter()
                .find(|k| k.device.index == index)
                .ok_or_else(|| {
                    DeviceError(format!(
                        "device {index} does not exist in this build (run `wb-engine info`)"
                    ))
                })?;
            let ordinal = k.gpu_ordinal.ok_or_else(|| {
                DeviceError(format!(
                    "device {index} ({}) is not a GPU",
                    k.device.description
                ))
            })?;
            Ok(Picked {
                gpu_ordinal: Some(ordinal),
                name: k.device.description.clone(),
                adapter: k.device.adapter_name.clone(),
            })
        }
    }
}

// ---------------------------------------------------------------------------- evaluation mode

/// The Phase 5a harness's settings JSON (`eval/wbeval/engines.py`, `Settings`).
fn settings_from_harness(v: &serde_json::Value) -> Settings {
    let mut s = Settings::default();
    let get = |k: &str| v.get(k);
    if let Some(x) = get("role").and_then(|x| x.as_str()) {
        s.role = x.to_string();
    }
    if let Some(x) = get("beam_size").and_then(|x| x.as_i64()) {
        s.beam_size = x as i32;
    }
    if let Some(x) = get("best_of").and_then(|x| x.as_i64()) {
        s.best_of = x as i32;
    }
    if let Some(x) = get("temperature_fallback").and_then(|x| x.as_bool()) {
        s.temperature_fallback = x;
    }
    if let Some(x) = get("previous_text_context").and_then(|x| x.as_bool()) {
        s.previous_text_context = x;
    }
    if let Some(x) = get("timestamps").and_then(|x| x.as_bool()) {
        s.timestamps = x;
    }
    if let Some(x) = get("glossary").and_then(|x| x.as_str()) {
        s.glossary = x.to_string();
    }
    if let Some(x) = get("max_chunk_s").and_then(|x| x.as_f64()) {
        s.max_chunk_s = x as f32;
    }
    if let Some(x) = get("flash_attn").and_then(|x| x.as_bool()) {
        s.flash_attn = x;
    }
    // Single-language evaluation merges speech greedily, as the harness's whisper-cli path does.
    s.sentence_split = false;
    s
}

fn find_models_dir(args: &[String]) -> PathBuf {
    if let Some(d) = args_value(args, "--models-dir") {
        return PathBuf::from(d);
    }
    if let Ok(d) = std::env::var("WB_MODELS_DIR") {
        return PathBuf::from(d);
    }
    let mut dir = std::env::current_dir().unwrap_or_default();
    for _ in 0..5 {
        let m = dir.join("models");
        if m.join("manifest.local.json").is_file() || m.join("manifest.json").is_file() {
            return m;
        }
        if !dir.pop() {
            break;
        }
    }
    PathBuf::from("models")
}

fn resolve_model(dir: &Path, id: &str) -> Result<ModelRef, Fail> {
    for name in ["manifest.local.json", "manifest.json"] {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        if let Ok(m) = wb_models::Manifest::load(&path)
            && let Ok(e) = m.find(id)
        {
            let file = dir.join(&e.file);
            if !file.is_file() {
                return Err(Fail {
                    code: exit::MODEL,
                    error_code: "model",
                    message: format!("model {id}: {} does not exist", file.display()),
                });
            }
            return Ok(ModelRef {
                id: id.to_string(),
                path: file.to_string_lossy().into_owned(),
            });
        }
    }
    Err(Fail {
        code: exit::MODEL,
        error_code: "model",
        message: format!("model {id} is not in a manifest under {}", dir.display()),
    })
}

fn eval_request(args: &[String]) -> Result<BatchRequest, Fail> {
    let file = args_value(args, "--file")
        .ok_or_else(|| Fail::usage("--file WAV is required without --request"))?;
    let models = args_value(args, "--models")
        .ok_or_else(|| Fail::usage("--models lang=id,... is required"))?;
    let langs: Vec<String> = args_value(args, "--langs")
        .ok_or_else(|| Fail::usage("--langs en,hi is required"))?
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if langs.is_empty() {
        return Err(Fail::usage("--langs lists no language"));
    }
    let harness: serde_json::Value = match args_value(args, "--settings") {
        Some(j) => serde_json::from_str(j)
            .map_err(|e| Fail::usage(format!("--settings is not valid JSON: {e}")))?,
        None => serde_json::Value::Null,
    };
    let settings = settings_from_harness(&harness);
    let dir = find_models_dir(args);
    let mut models_by_lang = BTreeMap::new();
    for pair in models.split(',').filter(|s| !s.is_empty()) {
        let (lang, id) = pair
            .split_once('=')
            .ok_or_else(|| Fail::usage(format!("--models entry '{pair}' is not lang=id")))?;
        models_by_lang.insert(lang.to_string(), resolve_model(&dir, id)?);
    }
    let vad_id = harness
        .get("vad_model")
        .and_then(|v| v.as_str())
        .unwrap_or("silero-v6.2.0");
    let vad_model = resolve_model(&dir, vad_id)?;
    let threads = harness
        .get("threads")
        .and_then(|v| v.as_u64())
        .map(|t| t as u32)
        .or_else(|| args_value(args, "--threads").and_then(|t| t.parse().ok()))
        .unwrap_or_else(|| wb_machine::probe().batch_threads() as u32);
    let device = match args_value(args, "--device") {
        None | Some("cpu") => Device::Cpu,
        Some(d) => match d.strip_prefix("gpu:").and_then(|n| n.parse().ok()) {
            Some(index) => Device::Gpu { index },
            None => return Err(Fail::usage("--device is cpu or gpu:N")),
        },
    };
    Ok(BatchRequest {
        source: Source::File {
            path: file.to_string(),
        },
        units: Vec::new(),
        models_by_lang,
        vad_model,
        detect_language: langs.len() > 1,
        languages: langs,
        lang_log: Vec::new(),
        settings,
        device,
        threads,
        resume_from_ms: 0,
    })
}

// ------------------------------------------------------------------------------------ commands

fn cmd_info() -> Result<(), Fail> {
    begin()?;
    out::emit(&Event::Info(Box::new(info::report()))).map_err(|e| Fail {
        code: exit::ERROR,
        error_code: "io",
        message: e.to_string(),
    })
}

fn cmd_batch(args: &[String]) -> Result<(), Fail> {
    begin()?;
    let (req, out_path) = if args.iter().any(|a| a == "--request") {
        match read_request(args)? {
            Request::Batch(b) => (b, None),
            other => {
                return Err(Fail::usage(format!(
                    "the request is a {other:?}, not a batch"
                )));
            }
        }
    } else {
        (
            eval_request(args)?,
            args_value(args, "--out").map(PathBuf::from),
        )
    };
    let mut file = match &out_path {
        Some(p) => Some(BufWriter::new(File::create(p).map_err(|e| {
            Fail::usage(format!("cannot create {}: {e}", p.display()))
        })?)),
        None => None,
    };
    let picked = pick_device(req.device).map_err(|e| Fail::from_error(&e))?;
    let mut emit = |e: &Event| -> std::io::Result<()> {
        out::emit(e)?;
        if let Some(f) = file.as_mut() {
            writeln!(f, "{}", e.to_line().map_err(std::io::Error::other)?)?;
            f.flush()?;
        }
        Ok(())
    };
    let mut sink = batch::Sink { emit: &mut emit };
    match batch::run(&req, picked.gpu_ordinal, &picked.name, &mut sink) {
        Ok(_) => Ok(()),
        Err(e) => {
            let f = Fail::from_error(&e);
            let _ = sink.send_error(f.error_code, &f.message);
            Err(f)
        }
    }
}

fn cmd_bench(args: &[String]) -> Result<(), Fail> {
    begin()?;
    let req: BenchRequest = match read_request(args)? {
        Request::Bench(b) => b,
        other => {
            return Err(Fail::usage(format!(
                "the request is a {other:?}, not a bench"
            )));
        }
    };
    let picked = pick_device(req.device).map_err(|e| Fail::from_error(&e))?;
    let report = bench::run(
        &req,
        picked.gpu_ordinal,
        &picked.name,
        picked.adapter.as_deref(),
    )
    .map_err(|e| Fail::from_error(&e))?;
    out::emit(&Event::Bench(Box::new(report))).map_err(|e| Fail {
        code: exit::ERROR,
        error_code: "io",
        message: e.to_string(),
    })
}

/// `chunks`: the chunks `batch` would decode for a file, from the VAD alone (no model, no language
/// detection). The code-switching gate fixes its chunks with this before anyone labels them.
fn cmd_chunks(args: &[String]) -> Result<(), Fail> {
    begin()?;
    let file = args_value(args, "--file").ok_or_else(|| Fail::usage("--file WAV is required"))?;
    let out_path = args_value(args, "--out").ok_or_else(|| Fail::usage("--out FILE.json is required"))?;
    let dir = find_models_dir(args);
    let vad_model = resolve_model(&dir, args_value(args, "--vad-model").unwrap_or("silero-v6.2.0"))?;
    let max_chunk_s: f32 = args_value(args, "--max-chunk-s").and_then(|v| v.parse().ok()).unwrap_or(15.0);
    let settings = Settings { max_chunk_s, ..Settings::default() };
    let pcm = audio::read_wav_16k(Path::new(file)).map_err(|e| Fail::from_error(&e))?;
    let mut vad = asr::Vad::load(&vad_model.path, 2).map_err(|e| Fail::from_error(&e))?;
    let speech = vad.speech(&pcm, &settings).map_err(|e| Fail::from_error(&e))?;
    let params = wb_engine_core::chunker::Params {
        max_chunk_s: f64::from(max_chunk_s),
        sentence_split: !args.iter().any(|a| a == "--greedy"),
        ..Default::default()
    };
    let chunks = wb_engine_core::chunker::chunks(&speech, &pcm, 16_000, &params);
    let list: Vec<serde_json::Value> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| serde_json::json!({ "chunk": i, "start_ms": c.start_ms, "end_ms": c.end_ms, "speech_ratio": c.speech_ratio() }))
        .collect();
    let doc = serde_json::json!({ "file": file, "duration_ms": pcm.len() as u64 / 16, "chunks": list });
    std::fs::write(out_path, serde_json::to_vec_pretty(&doc).map_err(|e| Fail::usage(e.to_string()))?)
        .map_err(|e| Fail::usage(format!("cannot write {out_path}: {e}")))?;
    out::emit(&Event::Done(wb_protocol::DoneSummary {
        segments: 0,
        chunks: chunks.len() as u32,
        audio_ms: pcm.len() as u64 / 16,
        truncated_chunks: 0,
        elapsed_s: 0.0,
        retried_chunks: 0,
        unclear_chunks: 0,
    }))
    .map_err(|e| Fail { code: exit::ERROR, error_code: "io", message: e.to_string() })
}

fn cmd_live(args: &[String]) -> Result<(), Fail> {
    begin()?;
    let req: LiveRequest = match read_request(args)? {
        Request::Live(l) => l,
        other => {
            return Err(Fail::usage(format!(
                "the request is a {other:?}, not a live request"
            )));
        }
    };
    live::run(&req).map_err(|e| Fail::from_error(&e))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("--version") | Some("-V") => {
            println!("{}", version_line());
            return ExitCode::SUCCESS;
        }
        Some("hello") => {
            let _ = out::emit(&hello());
            return ExitCode::SUCCESS;
        }
        Some("info") => cmd_info(),
        Some("batch") => cmd_batch(&args[1..]),
        Some("bench") => cmd_bench(&args[1..]),
        Some("live") => cmd_live(&args[1..]),
        Some("chunks") => cmd_chunks(&args[1..]),
        Some(other) => Err(Fail::usage(format!(
            "unknown command '{other}' (info, bench, batch, live)"
        ))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(f) => {
            out::log(&f.message);
            let _ = out::emit(&Event::Error {
                code: f.error_code.into(),
                message: f.message.clone(),
                fatal: true,
            });
            ExitCode::from(f.code)
        }
    }
}
