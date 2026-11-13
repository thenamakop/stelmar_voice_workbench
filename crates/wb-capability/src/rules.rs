//! The decision rules of PLAN section 3 and the Phase 3 plan (13 Nov 2026), as pure functions of the
//! measurements. Nothing here knows a model's speed in advance: every RTF comes from a `Measurement`.
//!
//! Bars (multiplied by 0.75 when the profile is emulated, LEGION-FIRST section 2): rolling RTF <= 1.0,
//! near-live RTF <= 0.35, tier G live stand-in RTF <= 0.25 (near-live settings on the GPU; Phase 7
//! replaces it with a streaming measurement). Memory: free RAM after a model's measured peak commit
//! (plus an integrated GPU's "local" memory, which is system RAM) stays at or above 3 GB.

use std::collections::BTreeMap;

use crate::types::*;

const ROLLING_BAR: f64 = 1.0;
const NEAR_LIVE_BAR: f64 = 0.35;
const LIVE_STANDIN_BAR: f64 = 0.25;
const HEADROOM: f64 = 0.75;

pub fn bars(p: &Profile) -> Bars {
    let m = if p.emulated.is_some() { HEADROOM } else { 1.0 };
    Bars {
        rolling_rtf: ROLLING_BAR * m,
        near_live_rtf: NEAR_LIVE_BAR * m,
        live_standin_rtf: LIVE_STANDIN_BAR * m,
        headroom_factor: m,
    }
}

/// Minutes after Stop at which a 90-minute talk's transcript is complete, with 10-minute parts:
/// max(10 + 90 RTF, 90 + 10 RTF) minutes after the talk starts, minus the 90 minutes of the talk.
pub fn ready_after_stop_min(rtf: f64) -> f64 {
    (f64::max(10.0 + 90.0 * rtf, 90.0 + 10.0 * rtf) - 90.0).max(0.0)
}

fn gpu_of(p: &Profile, index: u32) -> Option<&Gpu> {
    p.hardware.gpus.iter().find(|g| g.index == index)
}

fn find<'a>(
    p: &'a Profile,
    model: &str,
    lang: Lang,
    mode: Mode,
    config: DeviceConfig,
    standin: bool,
) -> Option<&'a Measurement> {
    p.measurements.iter().find(|m| {
        m.model == model
            && m.lang == lang
            && m.mode == mode
            && m.config == config
            && m.live_standin == standin
    })
}

/// Free RAM stays at or above the minimum after this measurement's peak (an integrated GPU's local
/// memory counts as RAM: DXGI reports it as local, not shared).
pub fn memory_ok(p: &Profile, m: &Measurement, o: &Options) -> bool {
    let igpu_local = match m.config.gpu_index().and_then(|i| gpu_of(p, i)) {
        Some(g) if g.kind == GpuKind::Integrated => m.gpu_local_bytes.unwrap_or(g.local_bytes),
        _ => 0,
    };
    let need = m.peak_commit_bytes.saturating_add(igpu_local);
    p.hardware.available_ram_bytes.saturating_sub(need) >= o.min_free_ram_bytes
}

/// Before measuring: could this model be loaded at all (file size x 1.3 plus the free-RAM minimum)?
pub fn model_fits(hw: &Hardware, model_bytes: u64, o: &Options) -> bool {
    let need = (model_bytes as f64 * 1.3) as u64;
    hw.available_ram_bytes.saturating_sub(need) >= o.min_free_ram_bytes
}

fn safety_of(p: &Profile, config: DeviceConfig) -> Option<&RecorderSafety> {
    p.recorder_safety.iter().find(|s| s.config == config)
}

/// The hybrid cell counts when it measured at least `hybrid_min_gain` faster than the best single
/// device for the same model and the recorder safety run of the hybrid configuration passed.
/// Returns the effective RTF of that model and language under the hybrid configuration.
fn hybrid_effective<'a>(p: &'a Profile, o: &Options) -> Option<(&'a HybridMeasurement, f64, f64)> {
    let h = p.hybrid.as_ref()?;
    let gpu_alone = find(
        p,
        &h.model,
        h.lang,
        Mode::Rolling,
        DeviceConfig::Gpu { index: h.gpu },
        false,
    )?;
    let cpu_alone = find(p, &h.model, h.lang, Mode::Rolling, DeviceConfig::Cpu, false);
    // The graphics engine alone on the hybrid test's own excerpt, when the test measured it; else the cell.
    let cpu_alone_throughput = cpu_alone.map_or(0.0, |c| 1.0 / c.rtf);
    let single_cell = f64::max(1.0 / gpu_alone.rtf, cpu_alone_throughput);
    let single = f64::max(
        1.0 / h.gpu_alone_rtf.unwrap_or(gpu_alone.rtf),
        cpu_alone_throughput,
    );
    let combined = h.combined_throughput();
    let gain = combined / single;
    let safe = safety_of(p, DeviceConfig::Hybrid { gpu: h.gpu }).is_some_and(|s| s.passed);
    // Two engines hold the model at once: both measured peaks, plus the integrated graphics memory, plus
    // the reserve must fit in the free RAM (AGENTS.md memory rule).
    let (gpu_peak, cpu_peak) = if h.gpu_peak_commit_bytes > 0 && h.cpu_peak_commit_bytes > 0 {
        (h.gpu_peak_commit_bytes, h.cpu_peak_commit_bytes)
    } else {
        (
            gpu_alone.peak_commit_bytes,
            cpu_alone.map_or(gpu_alone.peak_commit_bytes, |c| c.peak_commit_bytes),
        )
    };
    let igpu_local = match gpu_of(p, h.gpu) {
        Some(g) if g.kind == GpuKind::Integrated => h
            .gpu_local_bytes
            .or(gpu_alone.gpu_local_bytes)
            .unwrap_or(g.local_bytes),
        _ => 0,
    };
    let need = gpu_peak.saturating_add(cpu_peak).saturating_add(igpu_local);
    let fits = p.hardware.available_ram_bytes.saturating_sub(need) >= o.min_free_ram_bytes;
    if gain >= o.hybrid_min_gain && safe && fits {
        // The hybrid's effective speed is the single-device cell's speed times the measured gain, so it is
        // compared with the other cells on the same footing (the cell and the test can use different excerpts).
        Some((h, gain, 1.0 / (single_cell * gain)))
    } else {
        None
    }
}

fn candidate_configs(p: &Profile, o: &Options) -> Vec<DeviceConfig> {
    let mut out: Vec<DeviceConfig> = Vec::new();
    for m in &p.measurements {
        if m.mode != Mode::Rolling || m.live_standin {
            continue;
        }
        let ok = match m.config {
            DeviceConfig::Cpu => true,
            DeviceConfig::Gpu { index } => gpu_of(p, index).is_some_and(|g| g.vulkan_usable),
            DeviceConfig::Hybrid { .. } => false,
        };
        if ok && !out.contains(&m.config) {
            out.push(m.config);
        }
    }
    // Cpu first, then GPUs by index, so a tie is broken towards the simpler configuration.
    out.sort_by_key(|c| match c {
        DeviceConfig::Cpu => (0, 0),
        DeviceConfig::Gpu { index } => (1, *index),
        DeviceConfig::Hybrid { gpu } => (2, *gpu),
    });
    // A failed recorder safety run rules a configuration out.
    out.retain(|c| safety_of(p, *c).is_none_or(|s| s.passed));
    if let Some((h, _, _)) = hybrid_effective(p, o) {
        out.push(DeviceConfig::Hybrid { gpu: h.gpu });
    }
    out
}

fn rolling_pick(bar: f64, rk: &Ranked, rtf: f64) -> RollingPick {
    RollingPick {
        model: rk.model.clone(),
        rtf,
        meets_bar: rtf <= bar,
        quality_unverified: rk.quality_unverified,
        k_max: if rtf > 0.0 {
            (bar / rtf).floor() as u32
        } else {
            0
        },
        ready_after_stop_min: ready_after_stop_min(rtf),
        lags: rtf > 1.0,
    }
}

/// The rolling model of each language under one configuration: the first ranked model that meets the
/// bar and fits memory; if none does, the fastest measured one (the screens then show the lag).
fn rolling_picks(
    p: &Profile,
    r: &Rankings,
    o: &Options,
    b: &Bars,
    config: DeviceConfig,
) -> BTreeMap<Lang, RollingPick> {
    let hybrid = hybrid_effective(p, o);
    let measured_on = match config {
        DeviceConfig::Hybrid { gpu } => DeviceConfig::Gpu { index: gpu },
        c => c,
    };
    let mut out = BTreeMap::new();
    for lang in Lang::ALL {
        let Some(list) = r.rolling.get(&lang) else {
            continue;
        };
        let mut first_meeting: Option<RollingPick> = None;
        let mut fastest: Option<RollingPick> = None;
        for rk in list {
            let Some(m) = find(p, &rk.model, lang, Mode::Rolling, measured_on, false) else {
                continue;
            };
            if !memory_ok(p, m, o) {
                continue;
            }
            let mut rtf = m.rtf;
            if matches!(config, DeviceConfig::Hybrid { .. })
                && let Some((h, _, eff)) = hybrid
                && h.model == rk.model
                && h.lang == lang
            {
                rtf = rtf.min(eff);
            }
            let pick = rolling_pick(b.rolling_rtf, rk, rtf);
            if pick.meets_bar && first_meeting.is_none() {
                first_meeting = Some(pick.clone());
            }
            if fastest.as_ref().is_none_or(|f| pick.rtf < f.rtf) {
                fastest = Some(pick);
            }
        }
        if let Some(pick) = first_meeting.or(fastest) {
            out.insert(lang, pick);
        }
    }
    out
}

/// Higher is better: (languages meeting the bar, then the slowest chosen model's speed).
fn score(picks: &BTreeMap<Lang, RollingPick>) -> (usize, f64) {
    let meeting = picks.values().filter(|p| p.meets_bar).count();
    let worst = picks.values().map(|p| p.rtf).fold(0.0_f64, f64::max);
    (meeting, -worst)
}

fn config_label(p: &Profile, config: DeviceConfig) -> String {
    let gpu = |i: u32| gpu_of(p, i).map_or_else(|| format!("GPU {i}"), |g| g.name.clone());
    match config {
        DeviceConfig::Cpu => format!("CPU ({})", p.hardware.cpu_brand),
        DeviceConfig::Gpu { index } => format!("{} (Vulkan)", gpu(index)),
        DeviceConfig::Hybrid { gpu: i } => format!("CPU + {} (hybrid)", gpu(i)),
    }
}

fn cpu_threads_for(p: &Profile, config: DeviceConfig) -> u32 {
    // Post-processing: physical cores minus one in total across engines (PLAN section 3).
    let total = p.hardware.physical_cores.saturating_sub(1).max(1);
    match config {
        DeviceConfig::Cpu => total,
        // The GPU engine keeps one or two CPU threads of its own.
        DeviceConfig::Gpu { .. } => total.min(2),
        DeviceConfig::Hybrid { .. } => total.saturating_sub(2).max(1),
    }
}

fn night_picks(
    p: &Profile,
    r: &Rankings,
    o: &Options,
    config: DeviceConfig,
    rolling: &BTreeMap<Lang, RollingPick>,
) -> BTreeMap<Lang, (Option<NightPick>, Option<String>)> {
    let measured_on = match config {
        DeviceConfig::Hybrid { gpu } => DeviceConfig::Gpu { index: gpu },
        c => c,
    };
    let mut out = BTreeMap::new();
    for lang in Lang::ALL {
        let list = r.night.get(&lang).map(Vec::as_slice).unwrap_or(&[]);
        let mut any_measured = false;
        let mut pick = None;
        for rk in list {
            let Some(m) = find(p, &rk.model, lang, Mode::Rolling, measured_on, false) else {
                continue;
            };
            any_measured = true;
            let hours = m.rtf * o.day_audio_hours;
            if !memory_ok(p, m, o) || hours > o.night_window_hours {
                continue;
            }
            // The night pass is worth running only with a different model than the rolling one.
            if rolling.get(&lang).is_some_and(|rp| rp.model == rk.model) {
                continue;
            }
            pick = Some(NightPick {
                model: rk.model.clone(),
                rtf: m.rtf,
                hours_for_a_day: hours,
                quality_unverified: rk.quality_unverified,
            });
            break;
        }
        let note = if pick.is_some() {
            None
        } else if list.is_empty() {
            // The measured comparison found no night model clearly more accurate than the rolling one.
            Some(
                "No night pass for this language: the large model was not clearly more accurate than the rolling model, so the rolling result stands."
                    .to_string(),
            )
        } else if !any_measured {
            Some("The night model is measured by the full check.".to_string())
        } else {
            Some(
                "No night model both differs from the rolling model and finishes a day's recordings before 08:00 here."
                    .to_string(),
            )
        };
        out.insert(lang, (pick, note));
    }
    out
}

/// Tier G stand-in: the best near-live-settings measurement of a live candidate on a discrete GPU.
fn live_standin<'a>(
    p: &'a Profile,
    r: &Rankings,
    o: &Options,
    b: &Bars,
) -> Vec<(&'a Measurement, bool)> {
    let mut out = Vec::new();
    for m in p.measurements.iter().filter(|m| m.live_standin) {
        let discrete = m
            .config
            .gpu_index()
            .and_then(|i| gpu_of(p, i))
            .is_some_and(|g| g.kind == GpuKind::Discrete && g.vulkan_usable);
        if !discrete {
            continue;
        }
        let listed = r
            .live
            .get(&m.lang)
            .is_some_and(|l| l.iter().any(|x| x.model == m.model));
        let passes = m.rtf <= b.live_standin_rtf && memory_ok(p, m, o);
        out.push((m, listed && passes));
    }
    out
}

fn live_decision(
    p: &Profile,
    r: &Rankings,
    o: &Options,
    b: &Bars,
    lang: Lang,
    rolling: Option<&RollingPick>,
    standins: &[(&Measurement, bool)],
) -> LiveDecision {
    let mut soft: Vec<String> = Vec::new();
    let mut hard: Vec<String> = Vec::new();

    // Tier G: streaming-style live text on a working discrete GPU.
    if let Some((m, passes)) = standins.iter().find(|(m, _)| m.lang == lang) {
        // English is cleared on the measured turbo result (FLEURS 4.38 WER, owner's decision of 13 Nov 2026).
        // Hindi and Urdu live text on the graphics card needs a model checked for accuracy, as on the CPU.
        let validated = lang == Lang::En
            || r.live
                .get(&lang)
                .is_some_and(|l| l.iter().any(|x| x.model == m.model && x.validated));
        if *passes && validated && rolling.is_some_and(|rp| rp.meets_bar) {
            return LiveDecision {
                offered: true,
                kind: Some("streaming".into()),
                model: Some(m.model.clone()),
                rtf: Some(m.rtf),
                soft_reasons: vec![],
                hard_reasons: vec![],
            };
        }
        if !*passes {
            let text = format!(
                "The live test of {} on the graphics card ran at {:.2}× real time; live captions need {:.2} or better.",
                m.model, m.rtf, b.live_standin_rtf
            );
            // Slower than real time can never keep up: the override cannot lift that.
            if m.rtf >= 1.0 {
                hard.push(text);
            } else {
                soft.push(text);
            }
        } else if !validated {
            // The speed is fine; only the accuracy check is missing, so the CPU near-live path (whose
            // measurements do not matter on this machine) is not consulted. The override may lift it.
            return LiveDecision {
                offered: false,
                kind: None,
                model: None,
                rtf: Some(m.rtf),
                soft_reasons: vec![format!(
                    "No live model has been checked for accuracy in {} yet; on the graphics card only English is offered for now.",
                    lang.name()
                )],
                hard_reasons: vec![],
            };
        } else {
            soft.push(format!(
                "The rolling model for {} does not keep up here, so live captions would fall behind.",
                lang.name()
            ));
        }
    }

    // Tier C+: near-live text from a small model on the CPU.
    let ranked = r.near_live.get(&lang).map(Vec::as_slice).unwrap_or(&[]);
    if ranked.is_empty() {
        // Neither can be lifted by the override: there is no model to run.
        hard.push(if lang == Lang::Ur {
            "Urdu live text is never offered on a machine without a working graphics card (no small Urdu model is good enough).".to_string()
        } else {
            format!("No near-live model is listed for {}.", lang.name())
        });
    } else {
        let measured = ranked.iter().find_map(|rk| {
            find(p, &rk.model, lang, Mode::NearLive, DeviceConfig::Cpu, false).map(|m| (rk, m))
        });
        match measured {
            None if !ranked.iter().any(|rk| rk.validated) => hard.push(format!(
                "Not measured: no validated near-live model for {}.",
                lang.name()
            )),
            None => hard.push(format!(
                "The near-live speed for {} has not been measured yet.",
                lang.name()
            )),
            Some((rk, m)) => {
                if m.rtf >= 1.0 {
                    hard.push(format!(
                        "Near-live text for {} runs slower than real time here ({:.2}×), so it can never keep up.",
                        lang.name(),
                        m.rtf
                    ));
                } else if m.rtf > b.near_live_rtf {
                    soft.push(format!(
                        "Near-live text for {} is too slow on this machine: {:.2}× real time, the bar is {:.2}.",
                        lang.name(),
                        m.rtf,
                        b.near_live_rtf
                    ));
                }
                if m.truncated_chunks > 0 {
                    hard.push(format!(
                        "The near-live test for {} lost speech ({} truncated chunk{}).",
                        lang.name(),
                        m.truncated_chunks,
                        if m.truncated_chunks == 1 { "" } else { "s" }
                    ));
                }
                if !memory_ok(p, m, o) {
                    // A memory shortfall is the recorder's safety margin: never overridable.
                    hard.push(
                        "Not enough memory would stay free with the near-live model loaded."
                            .to_string(),
                    );
                }
                if !rk.validated {
                    soft.push(format!(
                        "No near-live model has been checked for accuracy in {} yet.",
                        lang.name()
                    ));
                }
                if soft.is_empty() && hard.is_empty() {
                    return LiveDecision {
                        offered: true,
                        kind: Some("near_live".into()),
                        model: Some(rk.model.clone()),
                        rtf: Some(m.rtf),
                        soft_reasons: vec![],
                        hard_reasons: vec![],
                    };
                }
            }
        }
    }
    LiveDecision {
        offered: false,
        kind: None,
        model: None,
        rtf: None,
        soft_reasons: soft,
        hard_reasons: hard,
    }
}

fn hinglish(p: &Profile, o: &Options, b: &Bars, config: DeviceConfig) -> Hinglish {
    let measured_on = match config {
        DeviceConfig::Hybrid { gpu } => DeviceConfig::Gpu { index: gpu },
        c => c,
    };
    let direct = p.measurements.iter().find(|m| {
        m.model.starts_with("hinglish")
            && m.lang == Lang::Hi
            && m.mode == Mode::Rolling
            && m.config == measured_on
            && !m.live_standin
    });
    let (m, source) = match direct {
        Some(m) => (Some(m), "measured".to_string()),
        None => (
            find(
                p,
                "large-v3-q5_0",
                Lang::Hi,
                Mode::Rolling,
                measured_on,
                false,
            ),
            "proxy: large-v3-q5_0 (same size)".to_string(),
        ),
    };
    match m {
        Some(m) => Hinglish {
            ok: m.rtf <= b.rolling_rtf && memory_ok(p, m, o),
            rtf: Some(m.rtf),
            source,
        },
        None => Hinglish {
            ok: false,
            rtf: None,
            source: "unmeasured".to_string(),
        },
    }
}

fn record_only_reason(p: &Profile) -> Option<String> {
    if !p.hardware.avx2 {
        return Some("This processor has no AVX2, which the speech engines need.".to_string());
    }
    if p.hardware.total_ram_bytes < 8 * GIB {
        return Some("This computer has less than 8 GB of memory.".to_string());
    }
    if !p.engines.iter().any(|e| e.usable) {
        let why = p
            .engines
            .iter()
            .filter_map(|e| e.reason.as_deref())
            .next()
            .unwrap_or("none found");
        return Some(format!("No speech engine works on this computer ({why})."));
    }
    None
}

/// Applies the rules to a profile. Never fails: a profile with nothing usable decides tier R.
pub fn decide(p: &Profile, r: &Rankings, o: &Options) -> Decision {
    let b = bars(p);
    let provisional = r.provisional;
    let mut reasons: Vec<String> = Vec::new();
    let mut languages: BTreeMap<Lang, LangDecision> = BTreeMap::new();

    let empty_lang = || LangDecision {
        rolling: None,
        night: None,
        night_note: None,
        live: LiveDecision {
            offered: false,
            kind: None,
            model: None,
            rtf: None,
            soft_reasons: vec![],
            hard_reasons: vec![],
        },
    };

    // Record only?
    let mut record_only = record_only_reason(p);

    // Best configuration.
    let mut best: Option<(DeviceConfig, BTreeMap<Lang, RollingPick>)> = None;
    if record_only.is_none() {
        for config in candidate_configs(p, o) {
            let picks = rolling_picks(p, r, o, &b, config);
            if picks.is_empty() {
                continue;
            }
            let better = best
                .as_ref()
                .is_none_or(|(_, bp)| score(&picks) > score(bp));
            if better {
                best = Some((config, picks));
            }
        }
        if best.is_none() {
            record_only = Some(
                "No speech model fits in memory or finished the speed test on this computer."
                    .to_string(),
            );
        }
    }

    let Some((config, picks)) = best.filter(|_| record_only.is_none()) else {
        for lang in Lang::ALL {
            languages.insert(lang, empty_lang());
        }
        let why = record_only.unwrap_or_else(|| "Nothing could be measured.".to_string());
        return Decision {
            tier: Tier::R,
            tier_reasons: vec![why.clone()],
            record_only_reason: Some(why.clone()),
            bars: b,
            config: None,
            languages,
            hinglish: Hinglish {
                ok: false,
                rtf: None,
                source: "unmeasured".into(),
            },
            dual_decode_en_hi: false,
            provisional,
            partial: p.check.partial,
            verdict: vec![
                "Record only.".to_string(),
                why,
                "Copy the session folder to a stronger computer to transcribe it.".to_string(),
            ],
            process_while_recording: true,
            pause_on_battery: true,
        };
    };

    // Per-language decisions under the chosen configuration.
    let nights = night_picks(p, r, o, config, &picks);
    let standins = live_standin(p, r, o, &b);
    let mut any_near_live = false;
    let mut any_streaming = false;
    for lang in Lang::ALL {
        let rolling = picks.get(&lang).cloned();
        let live = live_decision(p, r, o, &b, lang, rolling.as_ref(), &standins);
        if live.kind.as_deref() == Some("near_live") {
            any_near_live = true;
        }
        if live.offered && live.kind.as_deref() == Some("streaming") {
            any_streaming = true;
        }
        let (night, night_note) = nights.get(&lang).cloned().unwrap_or((None, None));
        languages.insert(
            lang,
            LangDecision {
                rolling,
                night,
                night_note,
                live,
            },
        );
    }

    // Tier.
    // Tier G: the live test passed on a graphics card and live captions are offered for at least one language.
    let g = any_streaming && standins.iter().any(|(_, passes)| *passes);
    let tier = if g {
        reasons.push("A graphics card works and the live test passed.".to_string());
        Tier::G
    } else if any_near_live {
        reasons.push(
            "A small model keeps up with near-live text for at least one language.".to_string(),
        );
        Tier::CPlus
    } else {
        reasons.push(
            "Rolling transcription works; live text is not offered on this machine.".to_string(),
        );
        Tier::C
    };

    let hing = hinglish(p, o, &b, config);
    let dual = match (picks.get(&Lang::En), picks.get(&Lang::Hi)) {
        (Some(en), Some(hi)) => en.rtf + hi.rtf <= b.rolling_rtf,
        _ => false,
    };

    let safety = safety_of(p, config).map(|s| s.passed);
    let why = match config {
        DeviceConfig::Cpu => "The CPU measured fastest for the rolling models.".to_string(),
        DeviceConfig::Gpu { .. } => {
            "The graphics card measured fastest for the rolling models.".to_string()
        }
        DeviceConfig::Hybrid { .. } => {
            let gain = hybrid_effective(p, o).map_or(0.0, |(_, g, _)| g);
            format!(
                "Running the CPU and the graphics card together measured {:.0}% faster than the best single device.",
                (gain - 1.0) * 100.0
            )
        }
    };
    let cfg = ConfigChoice {
        config,
        label: config_label(p, config),
        why,
        cpu_threads: cpu_threads_for(p, config),
        recorder_safety_passed: safety,
    };

    // Processing while the recording runs needs a recorder safety run that passed for this very
    // configuration (PLAN section 3). Without one the transcripts wait until Stop and the verdict says so.
    let safety_passed = safety == Some(true);
    let partial = p.check.partial
        || Lang::ALL.iter().take(2).any(|l| !picks.contains_key(l))
        || safety.is_none();
    let mut verdict = verdict_lines(&tier, &cfg, &languages, &hing, partial, provisional, &b);
    if !safety_passed {
        verdict.push(if safety.is_none() {
            "The recorder safety test has not run for this setup, so transcription waits until the recording stops.".to_string()
        } else {
            "The recorder safety test did not pass for this setup, so transcription waits until the recording stops.".to_string()
        });
    }

    Decision {
        tier,
        tier_reasons: reasons,
        record_only_reason: None,
        bars: b,
        config: Some(cfg),
        languages,
        hinglish: hing,
        dual_decode_en_hi: dual,
        provisional,
        partial,
        verdict,
        process_while_recording: safety_passed,
        pause_on_battery: true,
    }
}

fn verdict_lines(
    tier: &Tier,
    cfg: &ConfigChoice,
    languages: &BTreeMap<Lang, LangDecision>,
    hinglish: &Hinglish,
    partial: bool,
    provisional: bool,
    b: &Bars,
) -> Vec<String> {
    let mut v = Vec::new();
    v.push(match tier {
        Tier::G => {
            let offered: Vec<&str> = [(Lang::En, "English"), (Lang::Hi, "Hindi"), (Lang::Ur, "Urdu")]
                .iter()
                .filter(|(l, _)| languages.get(l).is_some_and(|d| d.live.offered))
                .map(|(_, n)| *n)
                .collect();
            match offered.len() {
                3 => "Record + post-process by default; live captions can be chosen for each recording.".to_string(),
                0 => "Record + post-process by default; live captions are not offered for any language yet.".to_string(),
                _ => format!(
                    "Record + post-process by default; live captions can be chosen for {} only.",
                    offered.join(" and ")
                ),
            }
        }
        Tier::CPlus => "Record + post-process by default; near-live text can be chosen for the languages listed below.".to_string(),
        _ => "Record + post-process. Live text is off by default and not offered on this machine.".to_string(),
    });
    v.push(format!("Transcribing runs on {}.", cfg.label));
    for lang in [Lang::En, Lang::Hi, Lang::Ur] {
        let Some(ld) = languages.get(&lang) else {
            continue;
        };
        let Some(rp) = &ld.rolling else { continue };
        let tag = if rp.quality_unverified {
            " (quality unverified)"
        } else {
            ""
        };
        if rp.lags {
            v.push(format!(
                "{}: {}{}. Processing will lag behind recording: about {:.0} minutes after the end of a 90-minute talk.",
                lang.name(), rp.model, tag, rp.ready_after_stop_min
            ));
        } else if !rp.meets_bar {
            v.push(format!(
                "{}: {}{}. It is within the {:.0}% safety margin of real time ({:.2}×, needs {:.2}); about {:.0} minutes after Stop.",
                lang.name(), rp.model, tag, (1.0 - b.headroom_factor) * 100.0, rp.rtf, b.rolling_rtf, rp.ready_after_stop_min
            ));
        } else {
            v.push(format!(
                "{}: {}{}, about {:.0} minutes after Stop.",
                lang.name(),
                rp.model,
                tag,
                rp.ready_after_stop_min
            ));
        }
    }
    v.push(if hinglish.ok {
        "The code-switching (Hinglish) model is fast enough to run on every chunk.".to_string()
    } else {
        "The code-switching (Hinglish) model is too slow to run on every chunk here.".to_string()
    });
    if partial {
        v.push("Some measurements are still to come: this verdict is partial.".to_string());
    }
    if provisional {
        v.push(
            "The model choice is provisional until the models have been checked for accuracy."
                .to_string(),
        );
    }
    v
}
