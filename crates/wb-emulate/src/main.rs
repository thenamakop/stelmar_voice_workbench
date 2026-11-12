//! wb-emulate: development only, never bundled. Runs a command inside a Windows Job Object that
//! makes the Legion stand in for a test laptop (docs/LEGION-FIRST.md section 2).
//!
//! ```text
//! wb-emulate --profile emu-low|emu-mid|native [--log FILE] -- <command> [args...]
//! wb-emulate probe [--commit-gb N] [--cuda-hold S]   what a child process sees (CPUs, commit, CUDA)
//! wb-emulate topology                  logical processors and their efficiency class
//! wb-emulate rig --profile emu-low|emu-mid   JSON: the job's mask and the two cores outside it
//!                                      the test rig (player, reference capture) is pinned to
//! ```
//!
//! Cores are chosen by efficiency class from GetLogicalProcessorInformationEx, never by assumed
//! numbering. Children inherit the job, its affinity mask and its 11 GiB commit limit; the job is
//! killed when this launcher exits. `CUDA_VISIBLE_DEVICES=-1` hides the NVIDIA GPU from CUDA and
//! `GGML_VK_VISIBLE_DEVICES` leaves only the Intel GPU to Vulkan.

#[cfg(windows)]
mod topology;
#[cfg(windows)]
mod win;

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    match app::run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("wb-emulate: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    eprintln!("wb-emulate runs on Windows only");
    ExitCode::FAILURE
}

#[cfg(windows)]
mod app {
    use std::io::Write;
    use std::process::{Command, ExitCode};

    use anyhow::{Context, Result, anyhow, bail};

    use crate::topology::{self, Core, Profile};
    use crate::win;

    const GIB: usize = 1 << 30;

    pub fn run() -> Result<ExitCode> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("probe") => probe(&args[1..]),
            Some("rig") => rig(&args[1..]),
            Some("topology") => {
                for c in topology::cores(&win::processor_core_records()?)? {
                    println!(
                        "core efficiency_class={} smt={} logical={:?}",
                        c.efficiency_class, c.smt, c.logical
                    );
                }
                Ok(ExitCode::SUCCESS)
            }
            _ => launch(&args),
        }
    }

    /// Prints the test rig's placement as JSON (masks as hex strings).
    fn rig(args: &[String]) -> Result<ExitCode> {
        let mut profile = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--profile" => {
                    profile = Some(
                        args.get(i + 1)
                            .ok_or_else(|| anyhow!("--profile needs a value"))?
                            .parse::<Profile>()?,
                    );
                    i += 2;
                }
                other => bail!(
                    "unexpected argument {other}; usage: wb-emulate rig --profile emu-low|emu-mid"
                ),
            }
        }
        let profile = profile.ok_or_else(|| anyhow!("--profile is required"))?;
        let cores = topology::cores(&win::processor_core_records()?)?;
        let Some(plan) = topology::rig(&cores, profile)? else {
            bail!("the native profile has no job mask; the rig is not pinned");
        };
        let json = serde_json::json!({
            "profile": profile.name(),
            "job_mask": format!("0x{:x}", plan.job_mask),
            "player_mask": format!("0x{:x}", plan.player_mask),
            "player_logical": plan.player_logical,
            "reference_mask": format!("0x{:x}", plan.reference_mask),
            "reference_logical": plan.reference_logical,
            "strict": plan.strict,
            "note": plan.note,
        });
        println!("{json}");
        Ok(ExitCode::SUCCESS)
    }

    fn launch(args: &[String]) -> Result<ExitCode> {
        let mut profile = None;
        let mut log_path = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--profile" => {
                    profile = Some(
                        args.get(i + 1)
                            .ok_or_else(|| anyhow!("--profile needs a value"))?
                            .parse::<Profile>()?,
                    );
                    i += 2;
                }
                "--log" => {
                    log_path = Some(
                        args.get(i + 1)
                            .ok_or_else(|| anyhow!("--log needs a value"))?
                            .clone(),
                    );
                    i += 2;
                }
                "--" => {
                    i += 1;
                    break;
                }
                other => bail!(
                    "unexpected argument {other}; usage: wb-emulate --profile emu-low|emu-mid|native -- <command>"
                ),
            }
        }
        let profile = profile.ok_or_else(|| anyhow!("--profile is required"))?;
        let command = args
            .get(i..)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| anyhow!("no command after --"))?;

        let cores = topology::cores(&win::processor_core_records()?)?;
        let selection = topology::select(&cores, profile)?;
        let mut lines = vec![
            format!("profile={}", profile.name()),
            format!("cores: {}", describe(&cores)),
        ];
        let mut env: Vec<(String, String)> = Vec::new();
        let job = match selection {
            Some(sel) => {
                lines.push(format!(
                    "affinity_mask=0x{:x} logical_processors={} chosen={:?} efficiency_classes={:?}",
                    sel.mask,
                    sel.mask.count_ones(),
                    sel.logical,
                    sel.classes
                ));
                lines.push(format!("job_memory_limit_bytes={}", 11 * GIB));
                env.push(("WB_EMULATE".into(), profile.name().into()));
                env.push((
                    "WB_EMULATE_REPORTED_RAM_BYTES".into(),
                    (16 * GIB).to_string(),
                ));
                env.push(("WB_EMULATE_JOB_MEMORY_BYTES".into(), (11 * GIB).to_string()));
                env.push((
                    "WB_EMULATE_LOGICAL_PROCESSORS".into(),
                    sel.mask.count_ones().to_string(),
                ));
                env.push(("CUDA_VISIBLE_DEVICES".into(), "-1".into()));
                match win::vulkan_devices() {
                    Ok(devs) => {
                        for d in &devs {
                            lines.push(format!(
                                "vulkan[{}] {} vendor=0x{:04x} integrated={} luid={} driver={}",
                                d.index,
                                d.name,
                                d.vendor_id,
                                d.integrated,
                                d.luid.as_deref().unwrap_or("unknown"),
                                d.driver
                            ));
                        }
                        // Only the first Intel integrated device: on the Legion the loader lists
                        // the same UHD GPU twice (two driver registrations), and exposing both
                        // would let ggml use one GPU as if it were two.
                        let intel: Vec<&win::VkDevice> = devs
                            .iter()
                            .filter(|d| d.vendor_id == 0x8086 && d.integrated)
                            .collect();
                        match intel.first() {
                            Some(first) => {
                                if intel.len() > 1 {
                                    let same = intel.iter().all(|d| d.luid.is_some() && d.luid == first.luid);
                                    lines.push(format!(
                                        "note: {} Intel integrated Vulkan devices listed ({}); exposing only index {}",
                                        intel.len(),
                                        if same { "same adapter LUID: one GPU listed twice" } else { "different LUIDs or unknown: check the drivers" },
                                        first.index
                                    ));
                                }
                                env.push(("GGML_VK_VISIBLE_DEVICES".into(), first.index.to_string()));
                            }
                            None => lines.push("warning: no Intel integrated Vulkan device; GGML_VK_VISIBLE_DEVICES not set".into()),
                        }
                    }
                    Err(e) => lines.push(format!(
                        "warning: Vulkan devices not listed ({e}); GGML_VK_VISIBLE_DEVICES not set"
                    )),
                }
                win::enter_job(Some(sel.mask), Some(11 * GIB))?
            }
            None => win::enter_job(None, None)?,
        };
        for (k, v) in &env {
            lines.push(format!("env {k}={v}"));
        }
        lines.push(format!("command: {}", command.join(" ")));
        for l in &lines {
            eprintln!("wb-emulate: {l}");
        }
        if let Some(path) = log_path {
            let mut f = std::fs::File::create(&path).with_context(|| format!("create {path}"))?;
            for l in &lines {
                writeln!(f, "{l}")?;
            }
        }
        let status = Command::new(&command[0])
            .args(&command[1..])
            .envs(env)
            .status()
            .with_context(|| format!("start {}", command[0]))?;
        // The launcher is inside the job itself (win.rs), and the job kills everything in it when its
        // handle closes. Closing it here would end this process before it returned the child's exit
        // code (every exit code read 0, found 10 Nov 2026); the handle closes when the process exits.
        std::mem::forget(job);
        Ok(ExitCode::from(
            u8::try_from(status.code().unwrap_or(1).clamp(0, 255)).unwrap_or(1),
        ))
    }

    fn describe(cores: &[Core]) -> String {
        let mut by_class = std::collections::BTreeMap::<u8, (usize, usize)>::new();
        for c in cores {
            let e = by_class.entry(c.efficiency_class).or_default();
            e.0 += 1;
            e.1 += c.logical.len();
        }
        by_class
            .iter()
            .map(|(k, (c, l))| format!("class {k}: {c} cores / {l} threads"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn probe(args: &[String]) -> Result<ExitCode> {
        let max_gb: usize = match args.iter().position(|a| a == "--commit-gb") {
            Some(p) => args
                .get(p + 1)
                .ok_or_else(|| anyhow!("--commit-gb needs a value"))?
                .parse()?,
            None => 16,
        };
        let mask = win::process_affinity()?;
        let cores = topology::cores(&win::processor_core_records()?)?;
        let mut classes = Vec::new();
        for c in &cores {
            for &l in &c.logical {
                if mask & (1usize << l) != 0 {
                    classes.push((l, c.efficiency_class));
                }
            }
        }
        classes.sort_unstable();
        let min_class = cores.iter().map(|c| c.efficiency_class).min().unwrap_or(0);
        let all_efficiency = classes.iter().all(|(_, k)| *k == min_class);
        println!("affinity_mask=0x{mask:x}");
        println!("logical_processors={}", mask.count_ones());
        println!(
            "std_available_parallelism={}",
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(0)
        );
        println!("logical_with_efficiency_class={classes:?}");
        println!("all_on_lowest_efficiency_class={all_efficiency}");
        for k in [
            "WB_EMULATE",
            "WB_EMULATE_REPORTED_RAM_BYTES",
            "WB_EMULATE_JOB_MEMORY_BYTES",
            "WB_EMULATE_LOGICAL_PROCESSORS",
            "CUDA_VISIBLE_DEVICES",
            "GGML_VK_VISIBLE_DEVICES",
        ] {
            println!(
                "env {k}={}",
                std::env::var(k).unwrap_or_else(|_| "(unset)".into())
            );
        }
        let hold = match args.iter().position(|a| a == "--cuda-hold") {
            Some(p) => Some(std::time::Duration::from_secs(
                args.get(p + 1)
                    .ok_or_else(|| anyhow!("--cuda-hold needs seconds"))?
                    .parse()?,
            )),
            None => None,
        };
        let cuda = win::cuda_probe(hold);
        println!("cuda_loaded={}", cuda.loaded);
        println!("cuda_init_result={}", cuda.init_result);
        println!("cuda_devices={}", cuda.devices);
        println!("cuda_context_held={}", cuda.context_held);
        let step = 256 << 20;
        let committed = win::probe_commit(step, max_gb * GIB);
        println!(
            "max_commit_bytes={committed} ({:.2} GiB, step 256 MiB, tried up to {max_gb} GiB)",
            committed as f64 / GIB as f64
        );
        Ok(ExitCode::SUCCESS)
    }
}
