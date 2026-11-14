//! wb-testbench: development and gate-pack tool for the recorder gates. Never bundled with the app.
//!
//! ```text
//! wb-testbench devices
//! wb-testbench gen      --out FILE.wav --seconds N [--rate 48000] [--channels 2] [--seed S]
//! wb-testbench mix      --out FILE.wav --clips A.wav,B.wav --seconds N [--rate 48000] [--pause-s 2] [--silent-from-s X --silent-for-s Y] [--gain G]
//!                       a speech source for the microphone run: the clips joined with pauses, repeated, with one stretch of true silence
//! wb-testbench play     --device "CABLE Input" --file FILE.wav [--stop-file F] [--go-file G] [--report R.json]
//! wb-testbench analyse  --session DIR --source FILE.wav [--report R.json]
//! wb-testbench decode   --session DIR [--report R.json]
//! wb-testbench parts-check --session DIR [--report R.json]   parts.json against the files
//! wb-testbench sessions [--device CABLE]            audio sessions playing into an endpoint
//! wb-testbench compare  --app SESSION --reference DIR [--reference-signal-start-ms N] [--app-signal-start-ms N] [--reference-tail ignore] [--report R.json]   fidelity: app vs reference
//! wb-testbench capture  --device "CABLE Output" --dir DIR --stop-file F [--go-file G] [--report R.json]
//!                       independent reference capture (to tell source-chain glitches from the recorder's)
//! wb-testbench capture-wasapi --device "Microphone" --dir DIR --stop-file F [--go-file G] [--report R.json]
//!                       the same capture through raw WASAPI (no cpal): the second reference
//! wb-testbench mute    --device "Microphone" [--flow capture|render] [--state on|off]   read or set an endpoint's mute (prints before and after)
//! wb-testbench volume  --device "Speakers" [--flow capture|render] [--level 0.0-1.0]   read or set an endpoint's master volume
//! wb-testbench scan-repeats --session DIR [--report R.json]   run the recorder's repeated-block detector over a recorded session
//! wb-testbench stress   [--threads N] [--gpu "UHD"] [--seconds N] [--stop-file F] [--report R.json]
//! ```
//!
//! Reports are JSON on stdout and, with `--report`, in a file written through the evidence
//! redaction rules (scripts/lib/Redact.psm1).

mod analyse;
mod compare;
mod mix;
mod play;
mod redact;
mod reference;
mod scan;
mod signal;
mod stress;
#[cfg(windows)]
mod win;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::HostTrait;
use serde::{Deserialize, Serialize};

struct Args {
    cmd: String,
    opts: HashMap<String, String>,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut it = std::env::args().skip(1);
        let cmd = it.next().ok_or_else(|| {
            anyhow!("usage: wb-testbench <devices|gen|play|analyse|decode|stress> [options]")
        })?;
        let mut opts = HashMap::new();
        while let Some(a) = it.next() {
            let key = a
                .strip_prefix("--")
                .ok_or_else(|| anyhow!("unexpected argument {a}"))?;
            let val = it.next().ok_or_else(|| anyhow!("--{key} needs a value"))?;
            opts.insert(key.to_string(), val);
        }
        Ok(Self { cmd, opts })
    }

    fn get(&self, k: &str) -> Result<&str> {
        self.opts
            .get(k)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("--{k} is required"))
    }

    fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> Result<T> {
        match self.opts.get(k) {
            Some(v) => v.parse().map_err(|_| anyhow!("--{k}: not a number: {v}")),
            None => Ok(default),
        }
    }
}

/// Sidecar written next to a generated file so the analyser knows the signal.
#[derive(Serialize, Deserialize)]
struct SourceInfo {
    seed: u64,
    sample_rate: u32,
    channels: u16,
    frames: u64,
}

fn sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".json");
    PathBuf::from(s)
}

fn output<T: Serialize>(args: &Args, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    println!("{text}");
    if let Some(path) = args.opts.get("report") {
        let mut body = redact::redact(&text);
        body.push('\n');
        std::fs::write(path, body).with_context(|| format!("write {path}"))?;
    }
    Ok(())
}

fn mix_source(args: &Args) -> Result<()> {
    let out = PathBuf::from(args.get("out")?);
    let clips: Vec<PathBuf> = args
        .get("clips")?
        .split(',')
        .map(|c| PathBuf::from(c.trim()))
        .filter(|c| !c.as_os_str().is_empty())
        .collect();
    let refs: Vec<&Path> = clips.iter().map(PathBuf::as_path).collect();
    let info = mix::write_mix(
        &out,
        &refs,
        args.num("seconds", 600)?,
        args.num("rate", 48_000)?,
        args.num("pause-s", 2.0)?,
        (
            args.num("silent-from-s", 0.0)?,
            args.num("silent-for-s", 0.0)?,
        ),
        args.num("gain", 1.0)?,
    )?;
    output(args, &info)
}

fn generate(args: &Args) -> Result<()> {
    let out = PathBuf::from(args.get("out")?);
    let seconds: u64 = args.num("seconds", 60)?;
    let rate: u32 = args.num("rate", 48_000)?;
    let channels: u16 = args.num("channels", 2)?;
    let seed: u64 = args.num("seed", signal::DEFAULT_SEED)?;
    let frames = seconds * u64::from(rate);
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&out, spec)?;
    for f in 0..frames {
        for c in 0..channels {
            w.write_sample(signal::sample(seed, f, c))?;
        }
    }
    w.finalize()?;
    let info = SourceInfo {
        seed,
        sample_rate: rate,
        channels,
        frames,
    };
    std::fs::write(sidecar(&out), serde_json::to_vec_pretty(&info)?)?;
    output(args, &info)
}

#[derive(Serialize)]
struct DeviceList {
    inputs: Vec<wb_recorder::capture::InputDevice>,
    outputs: Vec<String>,
    /// Windows' default playback device: any app's sound goes there, so it must not be the
    /// cable the gate plays into.
    default_output: Option<String>,
    /// Default playback device per Windows role (console, multimedia, communications).
    default_roles: Option<win::DefaultRender>,
    #[cfg(windows)]
    gpus: Vec<win::Adapter>,
    affinity_logical_processors: Option<u32>,
}

fn devices(args: &Args) -> Result<()> {
    let inputs = wb_recorder::capture::list_inputs()?;
    let host = cpal::default_host();
    let outputs = host.output_devices()?.map(|d| d.to_string()).collect();
    let default_output = host.default_output_device().map(|d| d.to_string());
    let list = DeviceList {
        inputs,
        outputs,
        default_output,
        default_roles: win::default_render_devices().ok(),
        #[cfg(windows)]
        gpus: win::adapters()?.into_iter().map(|(a, _)| a).collect(),
        affinity_logical_processors: affinity(),
    };
    output(args, &list)
}

fn affinity() -> Option<u32> {
    #[cfg(windows)]
    {
        win::affinity_count()
    }
    #[cfg(not(windows))]
    {
        std::thread::available_parallelism()
            .ok()
            .map(|n| n.get() as u32)
    }
}

#[derive(Serialize)]
struct DecodeCheck {
    file: String,
    header_ms: u64,
    header_matches_length: bool,
    decoded_ms: Option<u64>,
    error: Option<String>,
    playable: bool,
}

#[derive(Serialize)]
struct DecodeReport {
    decoder: &'static str,
    parts: Vec<DecodeCheck>,
    all_playable: bool,
}

fn decode(args: &Args) -> Result<()> {
    let session = PathBuf::from(args.get("session")?);
    let audio = session.join(wb_recorder::parts::AUDIO_DIR);
    let mut parts = Vec::new();
    for n in wb_recorder::parts::part_numbers_on_disk(&session)? {
        let path = audio.join(wb_recorder::parts::part_file_name(n));
        let info = wb_recorder::wav::inspect(&path);
        let header_frames = info.as_ref().map_or(0, |i| {
            u64::from(i.header_data_len) / u64::from(i.block_align.max(1))
        });
        let (header_ms, matches) = match &info {
            Ok(i) => (
                wb_recorder::wav::frames_to_ms(
                    u64::from(i.header_data_len) / u64::from(i.block_align),
                    i.sample_rate,
                ),
                i.header_matches_length(),
            ),
            Err(_) => (0, false),
        };
        #[cfg(windows)]
        let decoded = win::mf_decode_ms(&path);
        #[cfg(not(windows))]
        let decoded: Result<u64> = Err(anyhow!("Media Foundation needs Windows"));
        let (decoded_ms, error) = match decoded {
            Ok(ms) => (Some(ms), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        };
        // Playable: the decoder read it end to end and got the header's length (to 50 ms).
        let playable = decoded_ms.is_some_and(|d| d.abs_diff(header_ms) <= 50) && header_frames > 0;
        parts.push(DecodeCheck {
            file: wb_recorder::parts::part_rel_path(n),
            header_ms,
            header_matches_length: matches,
            decoded_ms,
            error,
            playable,
        });
    }
    let all_playable = !parts.is_empty() && parts.iter().all(|p| p.playable);
    output(
        args,
        &DecodeReport {
            decoder: "Media Foundation source reader",
            parts,
            all_playable,
        },
    )
}

#[derive(Serialize)]
struct PartsCheck {
    /// parts.json equals an index rebuilt from the part files alone (sizes, times, SHA-256).
    equal: bool,
    in_parts_json: usize,
    rebuilt: usize,
    differences: Vec<String>,
}

fn parts_check(args: &Args) -> Result<()> {
    let session = PathBuf::from(args.get("session")?);
    let written = wb_recorder::parts::load(&session)?;
    let rebuilt = wb_recorder::parts::rebuild_from_folder(&session)?;
    let mut differences = Vec::new();
    let n = written.parts.len().max(rebuilt.parts.len());
    for i in 0..n {
        match (written.parts.get(i), rebuilt.parts.get(i)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(a), Some(b)) => differences.push(format!(
                "part {}: parts.json {:?} vs files {:?}",
                a.part, a, b
            )),
            (Some(a), None) => {
                differences.push(format!("part {} is in parts.json but not on disk", a.part))
            }
            (None, Some(b)) => {
                differences.push(format!("part {} is on disk but not in parts.json", b.part))
            }
            (None, None) => {}
        }
    }
    output(
        args,
        &PartsCheck {
            equal: differences.is_empty(),
            in_parts_json: written.parts.len(),
            rebuilt: rebuilt.parts.len(),
            differences,
        },
    )
}

fn run() -> Result<()> {
    let args = Args::parse()?;
    match args.cmd.as_str() {
        "devices" => devices(&args),
        "gen" => generate(&args),
        "mix" => mix_source(&args),
        "play" => {
            let file = PathBuf::from(args.get("file")?);
            let stop = args.opts.get("stop-file").map(PathBuf::from);
            let go = args.opts.get("go-file").map(PathBuf::from);
            let report = play::play(args.get("device")?, &file, stop.as_deref(), go.as_deref())?;
            output(&args, &report)
        }
        "analyse" => {
            let source = PathBuf::from(args.get("source")?);
            let info: SourceInfo = serde_json::from_slice(
                &std::fs::read(sidecar(&source)).context("source sidecar .json")?,
            )?;
            let a = analyse::analyse(
                Path::new(args.get("session")?),
                info.seed,
                info.frames,
                info.sample_rate,
            )?;
            output(&args, &a)
        }
        "scan-repeats" => {
            let report = scan::scan(Path::new(args.get("session")?))?;
            output(&args, &report)
        }
        "lag-scan" => {
            let v = scan::lag_scan(
                Path::new(args.get("session")?),
                args.num("from-ms", 0.0)?,
                args.num("to-ms", 5000.0)?,
                args.num("min-lag", 1)?,
                args.num("max-lag", 2000)?,
            )?;
            output(&args, &v)
        }
        "frames" => {
            let v = scan::dump_frames(
                Path::new(args.get("session")?),
                args.num("at-ms", 0.0)?,
                args.num("count", 64)?,
            )?;
            output(&args, &v)
        }
        "decode" => decode(&args),
        "parts-check" => parts_check(&args),
        "compare" => {
            let app_dir = PathBuf::from(args.get("app")?);
            let app = compare::Recording::load(&app_dir)?;
            let reference = compare::Recording::load(Path::new(args.get("reference")?))?;
            let starts = compare::run_starts(&app_dir, &app).unwrap_or_else(|_| vec![0]);
            let rate = f64::from(app.rate);
            let from_ms = |key: &str| -> Option<u64> {
                args.opts
                    .get(key)
                    .and_then(|v| v.parse::<f64>().ok())
                    .map(|ms| (ms * rate / 1000.0).round() as u64)
            };
            let hints = compare::Hints {
                reference_signal_start: from_ms("reference-signal-start-ms"),
                app_signal_start: from_ms("app-signal-start-ms"),
                ignore_reference_tail: args
                    .opts
                    .get("reference-tail")
                    .is_some_and(|v| v == "ignore"),
            };
            let report = compare::compare(&app, &reference, &starts, &hints)?;
            output(&args, &report)
        }
        "capture" => {
            let report = reference::capture(
                args.get("device")?,
                Path::new(args.get("dir")?),
                Path::new(args.get("stop-file")?),
                args.opts.get("go-file").map(Path::new),
            )?;
            output(&args, &report)
        }
        "capture-wasapi" => {
            #[cfg(windows)]
            {
                let report = reference::capture_wasapi(
                    args.get("device")?,
                    Path::new(args.get("dir")?),
                    Path::new(args.get("stop-file")?),
                    args.opts.get("go-file").map(Path::new),
                )?;
                output(&args, &report)
            }
            #[cfg(not(windows))]
            bail!("capture-wasapi needs Windows")
        }
        "mute" => {
            #[cfg(windows)]
            {
                let flow = endpoint_flow(&args)?;
                let set = match args.opts.get("state").map(String::as_str) {
                    None => None,
                    Some("on") => Some(true),
                    Some("off") => Some(false),
                    Some(other) => bail!("--state is on or off, not {other}"),
                };
                let before = win::endpoint_mute(flow, args.get("device")?, None)?;
                let after = match set {
                    Some(_) => win::endpoint_mute(flow, args.get("device")?, set)?,
                    None => before.clone(),
                };
                output(
                    &args,
                    &MuteReport {
                        device: after.0,
                        was_muted: before.1,
                        muted: after.1,
                    },
                )
            }
            #[cfg(not(windows))]
            bail!("mute needs Windows")
        }
        "volume" => {
            #[cfg(windows)]
            {
                let flow = endpoint_flow(&args)?;
                let set = match args.opts.get("level") {
                    None => None,
                    Some(v) => Some(
                        v.parse::<f32>()
                            .ok()
                            .filter(|l| (0.0..=1.0).contains(l))
                            .ok_or_else(|| anyhow!("--level is a number from 0 to 1, not {v}"))?,
                    ),
                };
                let before = win::endpoint_level(flow, args.get("device")?, None)?;
                let after = match set {
                    Some(_) => win::endpoint_level(flow, args.get("device")?, set)?,
                    None => before.clone(),
                };
                output(
                    &args,
                    &VolumeReport {
                        device: after.0,
                        was_level: before.1,
                        level: after.1,
                    },
                )
            }
            #[cfg(not(windows))]
            bail!("volume needs Windows")
        }
        "sessions" => {
            #[cfg(windows)]
            {
                let list =
                    win::render_sessions(args.opts.get("device").map_or("CABLE", String::as_str))?;
                output(&args, &list)
            }
            #[cfg(not(windows))]
            bail!("sessions needs Windows")
        }
        "stress" => {
            let threads = match args.opts.get("threads") {
                Some(t) => t.parse().map_err(|_| anyhow!("--threads: not a number"))?,
                None => affinity().unwrap_or(1),
            };
            let stop_file = args.opts.get("stop-file").map(PathBuf::from);
            let report = stress::run(
                threads,
                args.opts.get("gpu").map(String::as_str),
                args.num("seconds", 1800)?,
                stop_file.as_deref(),
            )?;
            output(&args, &report)
        }
        other => bail!("unknown command {other}"),
    }
}

/// What `mute` reports: the state before and after, so a script can restore it.
#[derive(Serialize)]
struct MuteReport {
    device: String,
    was_muted: bool,
    muted: bool,
}

/// What `volume` reports: the level (0 to 1) before and after, so a script can restore it.
#[derive(Serialize)]
struct VolumeReport {
    device: String,
    was_level: f32,
    level: f32,
}

#[cfg(windows)]
fn endpoint_flow(args: &Args) -> Result<win::Flow> {
    match args.opts.get("flow").map_or("capture", String::as_str) {
        "capture" => Ok(win::Flow::Capture),
        "render" => Ok(win::Flow::Render),
        other => bail!("--flow is capture or render, not {other}"),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("wb-testbench: {e:#}");
            ExitCode::FAILURE
        }
    }
}
