//! The decision rules against fixture profiles (Phase 3 Verification): tier G laptop, tier C+ laptop
//! with a strong integrated GPU, tier C low-baseline laptop, tier R old PC, a machine whose avx512
//! build crashes, a machine whose integrated GPU is slower than its CPU (hybrid must not be chosen),
//! plus the headroom rule, hybrid rules, memory rule, lag estimate, k_max, overrides and fingerprint.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use common::*;
use wb_capability::*;

fn rolling_of(d: &Decision, lang: Lang) -> &RollingPick {
    d.languages[&lang].rolling.as_ref().expect("a rolling pick")
}

#[test]
fn tier_g_legion_uses_the_rtx_and_offers_live() {
    let d = decide_with(&legion_native(), false);
    assert_eq!(d.tier, Tier::G);
    assert_eq!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Gpu { index: 1 }
    );
    assert_eq!(d.bars.headroom_factor, 1.0);
    // English: medium is ranked first and meets the bar by a mile; Hindi: turbo first.
    assert_eq!(rolling_of(&d, Lang::En).model, "medium");
    assert_eq!(rolling_of(&d, Lang::Hi).model, "large-v3-turbo-q5_0");
    // Live on the graphics card: English only until Phase 5 has checked a Hindi and an Urdu live model.
    let en = &d.languages[&Lang::En].live;
    assert!(en.offered, "{:?}", en.soft_reasons);
    assert_eq!(en.kind.as_deref(), Some("streaming"));
    for lang in [Lang::Hi, Lang::Ur] {
        let live = &d.languages[&lang].live;
        assert!(!live.offered, "{lang:?}");
        assert!(
            live.hard_reasons.is_empty(),
            "{lang:?}: the override may lift this"
        );
        assert!(
            live.soft_reasons
                .iter()
                .any(|r| r.contains("checked for accuracy")),
            "{lang:?}: {:?}",
            live.soft_reasons
        );
    }
    // With models checked for accuracy (Phase 5) all three are offered.
    let v = decide_with(&legion_native(), true);
    for lang in Lang::ALL {
        let live = &v.languages[&lang].live;
        assert!(live.offered, "{lang:?}: {:?}", live.soft_reasons);
        assert_eq!(live.kind.as_deref(), Some("streaming"));
    }
    assert!(d.hinglish.ok, "large-v3-sized models are fast on the RTX");
    assert!(d.dual_decode_en_hi);
    assert!(rolling_of(&d, Lang::Hi).k_max >= 4);
    assert!(d.provisional);
}

#[test]
fn tier_g_needs_the_standin_to_pass() {
    let mut p = legion_native();
    for m in p.measurements.iter_mut().filter(|m| m.live_standin) {
        m.rtf = 0.4;
    }
    let d = decide_with(&p, false);
    assert_ne!(d.tier, Tier::G);
    assert!(!d.languages[&Lang::En].live.offered);
    assert!(d.languages[&Lang::En].live.soft_reasons[0].contains("live test"));
}

#[test]
fn tier_g_needs_live_offered_for_at_least_one_language() {
    // The live test passes, but English (the only language cleared on a graphics card for now) misses the
    // rolling bar, so no language is offered: the machine is not tier G.
    let mut p = legion_native();
    for m in p
        .measurements
        .iter_mut()
        .filter(|m| m.lang == Lang::En && m.mode == Mode::Rolling)
    {
        m.rtf = 1.6;
    }
    let d = decide_with(&p, false);
    assert!(!d.languages[&Lang::En].live.offered);
    assert_ne!(d.tier, Tier::G);
}

#[test]
fn the_hybrid_gain_is_compared_with_the_graphics_engine_alone_on_the_same_excerpt() {
    // The single-device cell was timed on a longer excerpt and reads faster than the hybrid test's own
    // excerpt does (the quick check times Hindi on the whole clip, the hybrid test on 30 s).
    let mut p = emu_mid_hybrid();
    let model = p.hybrid.as_ref().unwrap().model.clone();
    let lang = p.hybrid.as_ref().unwrap().lang;
    for m in p.measurements.iter_mut().filter(|m| {
        m.model == model
            && m.lang == lang
            && m.mode == Mode::Rolling
            && m.config == DeviceConfig::Gpu { index: 0 }
    }) {
        m.rtf = 0.5;
    }
    {
        let h = p.hybrid.as_mut().unwrap();
        h.gpu_rtf_concurrent = 0.8;
        h.cpu_rtf_concurrent = 2.0;
        h.gpu_alone_rtf = None;
    }
    // Against the cell alone (2.0 audio/s) the combined 1.75 is slower: not chosen.
    let d = decide_with(&p, false);
    assert!(!matches!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Hybrid { .. }
    ));
    // Against the engine alone on the same excerpt (0.7, 1.43 audio/s) the gain is 22%: chosen.
    p.hybrid.as_mut().unwrap().gpu_alone_rtf = Some(0.7);
    let d = decide_with(&p, false);
    assert!(matches!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Hybrid { .. }
    ));
}

#[test]
fn without_the_alone_run_a_cell_on_another_excerpt_is_not_a_baseline() {
    let mut p = emu_mid_hybrid();
    let model = p.hybrid.as_ref().unwrap().model.clone();
    let lang = p.hybrid.as_ref().unwrap().lang;
    {
        let h = p.hybrid.as_mut().unwrap();
        h.gpu_rtf_concurrent = 0.8;
        h.cpu_rtf_concurrent = 1.0;
        h.gpu_alone_rtf = None;
    }
    let set_cells = |p: &mut Profile, clip: f64| {
        for m in p
            .measurements
            .iter_mut()
            .filter(|m| m.model == model && m.lang == lang && m.mode == Mode::Rolling)
        {
            m.clip_seconds = clip;
        }
    };
    let is_hybrid = |p: &Profile| {
        matches!(
            decide_with(p, false).config.as_ref().map(|c| c.config),
            Some(DeviceConfig::Hybrid { .. })
        )
    };
    // The same excerpt as the hybrid test: the cell is a fair baseline.
    let excerpt = p.check.excerpt_seconds;
    set_cells(&mut p, excerpt);
    p.measurements
        .iter_mut()
        .filter(|m| {
            m.model == model && m.lang == lang && m.config == DeviceConfig::Gpu { index: 0 }
        })
        .for_each(|m| m.rtf = 1.0);
    assert!(is_hybrid(&p), "same footing, 1.0 + 0.8 audio/s ... gain");
    // A cell on a longer excerpt cannot be compared: the hybrid is not counted.
    set_cells(&mut p, excerpt * 2.0);
    assert!(!is_hybrid(&p));
}

#[test]
fn an_override_cannot_lift_a_missing_recorder_safety_run_even_where_live_was_only_softly_blocked() {
    // Strong iGPU, near-live not validated: only a soft reason, which the override lifts while the safety run
    // exists (see the test above). With no recorder safety run it must not.
    let mut p = strong_igpu_laptop();
    p.recorder_safety.clear();
    let mut c = cap(p, false);
    set_override(&mut c, OverrideKind::AllowLiveAnyway, true, true, "t");
    let plan = plan_session(&c, &[Lang::En]);
    assert!(!plan.live.selectable, "{:?}", plan.live);
    assert!(
        plan.live
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("safety test has not run"),
        "{:?}",
        plan.live.reason
    );
    assert!(!c.decision.process_while_recording);
}

#[test]
fn live_on_the_graphics_card_is_ruled_out_by_a_failed_graphics_safety_run() {
    // A discrete card whose own recorder safety run failed while the processor passed and keeps up: the
    // processor is chosen for the rolling models, the live test on the card passes, and streaming live text on
    // that card is still not offered; the override cannot lift it.
    let mut p = legion_native();
    let card = DeviceConfig::Gpu { index: 1 };
    for m in p
        .measurements
        .iter_mut()
        .filter(|m| m.config == DeviceConfig::Cpu)
    {
        m.rtf = 0.3;
    }
    p.measurements.push(rolling(
        "large-v3-turbo-q5_0",
        Lang::En,
        DeviceConfig::Cpu,
        0.3,
        0.91,
        None,
    ));
    assert!(
        decide_with(&p, false).languages[&Lang::En].live.offered,
        "with every safety run passed, English live is offered"
    );
    for s in &mut p.recorder_safety {
        if s.config == card {
            s.passed = false;
            s.overruns = 3;
        }
    }
    let d = decide_with(&p, false);
    assert_eq!(d.config.as_ref().unwrap().config, DeviceConfig::Cpu);
    let en = &d.languages[&Lang::En];
    assert!(!en.live.offered, "{en:?}");
    assert!(
        en.live
            .hard_reasons
            .iter()
            .any(|r| r.contains("graphics card")),
        "{en:?}"
    );
    let mut c = cap(p, false);
    set_override(&mut c, OverrideKind::AllowLiveAnyway, true, true, "t");
    assert!(!plan_session(&c, &[Lang::En]).live.selectable);
}

#[test]
fn record_only_processes_nothing_while_recording() {
    let d = decide_with(&old_pc(), false);
    assert!(d.config.is_none());
    assert!(!d.process_while_recording);
}

#[test]
fn processing_while_recording_override_lapses_when_a_later_check_did_not_pass_the_safety_test() {
    let mut c = cap(emu_low(), false);
    assert!(
        c.decision.process_while_recording,
        "the fixture passed its safety run"
    );
    c.overrides.push(OverrideRecord {
        kind: OverrideKind::ProcessWhileRecording,
        enabled: true,
        at: "2026-11-14T03:00:00+05:30".into(),
        confirmed: true,
    });
    assert!(override_applies(&c, OverrideKind::ProcessWhileRecording));
    // A later check on the same profile: the recorder safety test failed or did not run.
    c.decision.process_while_recording = false;
    assert!(
        override_enabled(&c.overrides, OverrideKind::ProcessWhileRecording),
        "the log still says the operator turned it on"
    );
    assert!(!override_applies(&c, OverrideKind::ProcessWhileRecording));
}

#[test]
fn tier_c_plus_strong_igpu_offers_near_live_only_where_validated() {
    // No near-live model is validated yet : speed alone does not offer live text.
    let d = decide_with(&strong_igpu_laptop(), false);
    assert_eq!(d.tier, Tier::C);
    let en = &d.languages[&Lang::En].live;
    assert!(!en.offered);
    assert!(
        en.soft_reasons
            .iter()
            .any(|r| r.contains("checked for accuracy"))
    );
    assert!(en.hard_reasons.is_empty());

    // Validated: English and Hindi near-live are offered, Urdu never.
    let d = decide_with(&strong_igpu_laptop(), true);
    assert_eq!(d.tier, Tier::CPlus);
    assert_eq!(
        d.languages[&Lang::En].live.kind.as_deref(),
        Some("near_live")
    );
    assert!(d.languages[&Lang::Hi].live.offered);
    let ur = &d.languages[&Lang::Ur].live;
    assert!(!ur.offered);
    assert!(ur.hard_reasons[0].contains("Urdu"));
}

#[test]
fn tier_c_low_baseline_follows_speed_table_a() {
    let d = decide_with(&emu_low(), true);
    assert_eq!(d.tier, Tier::C);
    assert!(
        d.bars.rolling_rtf < 0.76 && d.bars.rolling_rtf > 0.74,
        "25% headroom under emulation"
    );
    // The integrated GPU is essential: CPU-only small Hindi is 1.629, the UHD does turbo in 0.734.
    assert_eq!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Gpu { index: 0 }
    );
    let en = rolling_of(&d, Lang::En);
    assert_eq!((en.model.as_str(), en.rtf), ("medium", 0.354));
    assert!(en.meets_bar);
    let hi = rolling_of(&d, Lang::Hi);
    assert_eq!((hi.model.as_str(), hi.rtf), ("large-v3-turbo-q5_0", 0.734));
    assert!(
        hi.meets_bar,
        "0.734 is inside the emulated bar of 0.75, by 2%"
    );
    assert_eq!(hi.k_max, 1);
    assert_eq!(en.k_max, 2);
    assert!(
        (hi.ready_after_stop_min - 7.34).abs() < 0.05,
        "{}",
        hi.ready_after_stop_min
    );
    // No room for two decodes per chunk in Hindi, no Hinglish on every chunk (large-v3 Hindi 1.532).
    assert!(!d.dual_decode_en_hi);
    assert!(!d.hinglish.ok);
    // Near-live: slower than real time, a hard block that no override lifts (small Hindi 2.35).
    for lang in [Lang::En, Lang::Hi] {
        let live = &d.languages[&lang].live;
        assert!(!live.offered);
        assert!(
            !live.hard_reasons.is_empty(),
            "{lang:?} runs slower than real time"
        );
    }
}

#[test]
fn medium_hindi_on_the_uhd_misses_the_emulated_bar() {
    // Table A: medium Hindi on the UHD is 0.929: over 0.75, so the turbo model is the pick (not medium).
    let mut p = emu_low();
    p.measurements
        .retain(|m| !(m.model == "large-v3-turbo-q5_0" && m.lang == Lang::Hi));
    let d = decide_with(&p, false);
    let hi = rolling_of(&d, Lang::Hi);
    assert_ne!(
        hi.model, "medium",
        "medium Hindi misses the bar; small is faster and meets it"
    );
    assert_eq!(hi.model, "small");
}

#[test]
fn the_headroom_rule_applies_only_to_emulated_profiles() {
    let mut p = legion_native();
    p.measurements
        .retain(|m| m.config == DeviceConfig::Gpu { index: 1 } && m.model == "large-v3-q5_0");
    p.measurements.iter_mut().for_each(|m| m.rtf = 0.80);
    let mut r = rankings(false);
    r.rolling.insert(
        Lang::Hi,
        vec![Ranked {
            model: "large-v3-q5_0".into(),
            bytes: 1,
            quality_unverified: true,
            validated: false,
        }],
    );
    let native = decide(&p, &r, &Options::default());
    assert!(
        native.languages[&Lang::Hi]
            .rolling
            .as_ref()
            .unwrap()
            .meets_bar
    );
    p.emulated = Some("emu-low".into());
    let emulated = decide(&p, &r, &Options::default());
    assert!(
        !emulated.languages[&Lang::Hi]
            .rolling
            .as_ref()
            .unwrap()
            .meets_bar
    );
}

#[test]
fn cpu_only_weak_laptop_gets_a_lag_estimate() {
    let d = decide_with(&cpu_only_weak(), false);
    assert_eq!(d.tier, Tier::C);
    assert_eq!(d.config.as_ref().unwrap().config, DeviceConfig::Cpu);
    let hi = rolling_of(&d, Lang::Hi);
    // Nothing meets the bar: the fastest ranked model (small, 1.629) with a "lags" flag.
    assert_eq!(hi.model, "small");
    assert!(!hi.meets_bar && hi.lags);
    assert!(
        (hi.ready_after_stop_min - 66.6).abs() < 0.2,
        "{}",
        hi.ready_after_stop_min
    );
    assert!(
        d.verdict.iter().any(|l| l.contains("lag behind recording")),
        "{:#?}",
        d.verdict
    );
    assert_eq!(rolling_of(&d, Lang::En).model, "small");
}

#[test]
fn tier_r_old_pc_records_only() {
    let d = decide_with(&old_pc(), false);
    assert_eq!(d.tier, Tier::R);
    assert!(d.config.is_none());
    assert!(d.record_only_reason.as_deref().unwrap().contains("AVX2"));
    let plan = plan_session(&cap(old_pc(), false), &[Lang::En]);
    assert!(plan.record_only);
    assert!(!plan.live.selectable);
}

#[test]
fn tier_r_under_8_gb_of_memory() {
    let mut p = strong_igpu_laptop();
    p.hardware.total_ram_bytes = gib(6.0);
    assert_eq!(decide_with(&p, false).tier, Tier::R);
}

#[test]
fn tier_r_when_no_model_fits_in_memory() {
    let mut p = emu_low();
    p.hardware.available_ram_bytes = gib(3.5);
    let d = decide_with(&p, false);
    assert_eq!(d.tier, Tier::R, "{:?}", d.record_only_reason);
}

#[test]
fn a_crashing_avx512_build_does_not_stop_the_avx2_build() {
    let d = decide_with(&avx512_crash(), false);
    assert_eq!(d.tier, Tier::C);
    assert_eq!(d.config.as_ref().unwrap().config, DeviceConfig::Cpu);
    let p = avx512_crash();
    let bad = p.engines.iter().find(|e| e.variant == "avx512").unwrap();
    assert!(
        !bad.usable && bad.reason.is_some(),
        "the crash is recorded with its reason"
    );
    assert!(rolling_of(&d, Lang::En).meets_bar);
}

#[test]
fn every_engine_failing_records_only() {
    let mut p = avx512_crash();
    for e in &mut p.engines {
        e.usable = false;
    }
    let d = decide_with(&p, false);
    assert_eq!(d.tier, Tier::R);
    assert!(
        d.record_only_reason
            .as_deref()
            .unwrap()
            .contains("No speech engine works")
    );
}

#[test]
fn an_igpu_slower_than_the_cpu_is_not_chosen_and_hybrid_is_not_either() {
    let d = decide_with(&slow_igpu(), false);
    assert_eq!(d.config.as_ref().unwrap().config, DeviceConfig::Cpu);
    // The hybrid cell measured 1/2.0 + 1/1.4 = 1.21 audio-s/s against the CPU alone at 1/0.55 = 1.82.
    assert!(!matches!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Hybrid { .. }
    ));
}

#[test]
fn hybrid_is_chosen_only_if_measured_faster_and_safe() {
    // emu-mid: UHD alone 0.659, together with the CPU 1/0.70 + 1/2.30 = 1.863 against 1.517: +22.8%.
    let d = decide_with(&emu_mid_hybrid(), false);
    assert_eq!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Hybrid { gpu: 0 }
    );
    let hi = rolling_of(&d, Lang::Hi);
    assert!((hi.rtf - 1.0 / 1.863).abs() < 0.01, "{}", hi.rtf);
    assert!(d.config.as_ref().unwrap().why.contains("faster"));

    // A gain of 12% (above the old 10% rule, below the 15% rule): GPU alone; a gain of 20%: hybrid.
    for (target, hybrid) in [(1.12, false), (1.20, true)] {
        let mut p = emu_mid_hybrid();
        let gpu_alone = p
            .measurements
            .iter()
            .find(|m| {
                m.lang == p.hybrid.as_ref().unwrap().lang
                    && m.config == DeviceConfig::Gpu { index: 0 }
                    && m.mode == Mode::Rolling
                    && m.model == p.hybrid.as_ref().unwrap().model
            })
            .unwrap()
            .rtf;
        let h = p.hybrid.as_mut().unwrap();
        let single = 1.0 / gpu_alone;
        h.cpu_rtf_concurrent = 1.0 / (target * single - 1.0 / h.gpu_rtf_concurrent);
        let d = decide_with(&p, false);
        let chosen = matches!(
            d.config.as_ref().unwrap().config,
            DeviceConfig::Hybrid { .. }
        );
        assert_eq!(chosen, hybrid, "gain {target}");
    }

    // Gain under 10%: GPU alone.
    let mut p = emu_mid_hybrid();
    p.hybrid.as_mut().unwrap().cpu_rtf_concurrent = 20.0;
    let d = decide_with(&p, false);
    assert_eq!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Gpu { index: 0 }
    );

    // Recorder safety failed for the hybrid configuration: not chosen.
    let mut p = emu_mid_hybrid();
    p.recorder_safety
        .iter_mut()
        .find(|s| matches!(s.config, DeviceConfig::Hybrid { .. }))
        .unwrap()
        .passed = false;
    p.recorder_safety
        .iter_mut()
        .find(|s| matches!(s.config, DeviceConfig::Hybrid { .. }))
        .unwrap()
        .overruns = 3;
    let d = decide_with(&p, false);
    assert_eq!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Gpu { index: 0 }
    );

    // No recorder safety run for it at all (a partial check): not chosen.
    let mut p = emu_mid_hybrid();
    p.recorder_safety
        .retain(|s| !matches!(s.config, DeviceConfig::Hybrid { .. }));
    assert_eq!(
        decide_with(&p, false).config.as_ref().unwrap().config,
        DeviceConfig::Gpu { index: 0 }
    );
}

#[test]
fn a_configuration_that_failed_the_recorder_safety_run_is_ruled_out() {
    let mut p = emu_low();
    p.recorder_safety = vec![RecorderSafety {
        config: DeviceConfig::Gpu { index: 0 },
        duration_s: 120.0,
        overruns: 2,
        driver_xruns: 0,
        passed: false,
    }];
    let d = decide_with(&p, false);
    assert_eq!(d.config.as_ref().unwrap().config, DeviceConfig::Cpu);
}

#[test]
fn memory_counts_the_integrated_gpus_local_memory_as_ram() {
    // medium on the UHD: commit 2.75 GiB + DXGI local 2.10 GiB = 4.85 GiB; with 7.5 GiB available
    // 2.65 GiB would stay free: under the 3 GB rule, so medium is skipped (turbo and small remain).
    let mut p = emu_low();
    p.hardware.available_ram_bytes = gib(7.5);
    let d = decide_with(&p, false);
    assert_ne!(rolling_of(&d, Lang::En).model, "medium");
    // The same machine with 8.0 GiB available keeps medium.
    p.hardware.available_ram_bytes = gib(8.0);
    assert_eq!(
        rolling_of(&decide_with(&p, false), Lang::En).model,
        "medium"
    );
}

#[test]
fn night_pass_follows_the_time_rule() {
    // Legion: large-v3 q5_0 English at 0.061 x 3 h of audio fits any night.
    let d = decide_with(&legion_native(), false);
    let night = d.languages[&Lang::Hi].night.as_ref().expect("a night pick");
    assert_eq!(night.model, "large-v3-q5_0");
    assert!(night.hours_for_a_day < 1.0);
    // emu-low CPU-only: 12 h window / 3 h of audio allows RTF 4.0; medium Hindi at 5.2 does not fit.
    let mut p = cpu_only_weak();
    p.measurements.push(rolling(
        "large-v3-q5_0",
        Lang::Hi,
        DeviceConfig::Cpu,
        7.113,
        2.53,
        None,
    ));
    let d = decide_with(&p, false);
    assert!(d.languages[&Lang::Hi].night.is_none());
    assert!(
        d.languages[&Lang::Hi]
            .night_note
            .as_deref()
            .unwrap()
            .contains("before 08:00")
    );
    // Unmeasured night candidates say so.
    let d = decide_with(&emu_low(), false);
    assert!(
        d.languages[&Lang::En]
            .night_note
            .as_deref()
            .unwrap()
            .contains("full check")
    );
}

#[test]
fn ready_time_formula_matches_plan_section_4() {
    assert!((ready_after_stop_min(0.5) - 5.0).abs() < 1e-9);
    assert!((ready_after_stop_min(1.0) - 10.0).abs() < 1e-9);
    assert!((ready_after_stop_min(1.5) - 55.0).abs() < 1e-9);
    assert!((ready_after_stop_min(2.0) - 100.0).abs() < 1e-9);
}

#[test]
fn k_max_is_how_many_decodes_fit_the_bar() {
    let d = decide_with(&emu_low(), false);
    assert_eq!(rolling_of(&d, Lang::En).k_max, 2); // floor(0.75 / 0.354)
    assert_eq!(rolling_of(&d, Lang::Hi).k_max, 1); // floor(0.75 / 0.734)
}

#[test]
fn hinglish_uses_its_own_cell_or_the_same_size_proxy() {
    let mut p = emu_low();
    let d = decide_with(&p, false);
    assert_eq!(d.hinglish.source, "proxy: large-v3-q5_0 (same size)");
    assert!(!d.hinglish.ok);
    p.measurements.push(rolling(
        "hinglish-q8_0",
        Lang::Hi,
        DeviceConfig::Gpu { index: 0 },
        0.6,
        2.7,
        Some(2.0),
    ));
    let d = decide_with(&p, false);
    assert_eq!(d.hinglish.source, "measured");
    assert!(d.hinglish.ok);
}

#[test]
fn the_hinglish_model_is_chosen_only_when_its_own_cell_meets_the_bar_and_the_verdict_says_what_is_kept()
 {
    let mut p = emu_low();
    // Only the same-size proxy: never chosen, and the verdict says it was not measured.
    let d = decide_with(&p, false);
    assert_eq!(d.hinglish.chosen_model(), None);
    assert_eq!(d.hinglish.model, None);
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("not measured on this computer") && l.contains("keep")),
        "{:#?}",
        d.verdict
    );
    // Its own cell, too slow even for the night: not chosen; today's models stay and the verdict names the cost.
    p.measurements.push(rolling(
        "hinglish-q8_0",
        Lang::Hi,
        DeviceConfig::Gpu { index: 0 },
        5.0,
        4.0,
        Some(3.0),
    ));
    let d = decide_with(&p, false);
    assert!(!d.hinglish.ok);
    assert_eq!(d.hinglish.model.as_deref(), Some("hinglish-q8_0"));
    assert_eq!(d.hinglish.chosen_model(), None);
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("too slow to run on every chunk") && l.contains("keep")),
        "{:#?}",
        d.verdict
    );
    // Its own cell, fast enough: chosen.
    p.measurements.retain(|m| m.model != "hinglish-q8_0");
    p.measurements.push(rolling(
        "hinglish-q8_0",
        Lang::Hi,
        DeviceConfig::Gpu { index: 0 },
        0.6,
        2.7,
        Some(2.0),
    ));
    let d = decide_with(&p, false);
    assert!(d.hinglish.ok, "{:?}", d.hinglish);
    assert_eq!(d.hinglish.chosen_model(), Some("hinglish-q8_0"));
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("use the code-switching (Hinglish) model hinglish-q8_0")),
        "{:#?}",
        d.verdict
    );
}

#[test]
fn a_hinglish_model_too_slow_for_the_recording_may_still_run_at_night_and_the_verdict_says_so() {
    let mut p = emu_low();
    // 1.4x real time: over the rolling bar, but 3 h of audio take 4.2 h of the 12-hour night window.
    p.measurements.push(rolling(
        "hinglish-q8_0",
        Lang::Hi,
        DeviceConfig::Gpu { index: 0 },
        1.4,
        4.0,
        Some(3.0),
    ));
    let d = decide_with(&p, false);
    assert!(!d.hinglish.ok);
    assert!(d.hinglish.night_ok, "{:?}", d.hinglish);
    assert_eq!(d.hinglish.night_model(), Some("hinglish-q8_0"));
    assert_eq!(
        d.hinglish.chosen_model(),
        None,
        "the recording keeps today's models"
    );
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("night pass decodes them again")
                && l.contains("hinglish-q8_0")
                && l.contains("best finished on a faster machine")),
        "{:#?}",
        d.verdict
    );
    // 5x real time: 15 h for 3 h of audio, over the window: no night model either, and the verdict marks it.
    p.measurements.retain(|m| m.model != "hinglish-q8_0");
    p.measurements.push(rolling(
        "hinglish-q8_0",
        Lang::Hi,
        DeviceConfig::Gpu { index: 0 },
        5.0,
        4.0,
        Some(3.0),
    ));
    let d = decide_with(&p, false);
    assert!(!d.hinglish.ok && !d.hinglish.night_ok, "{:?}", d.hinglish);
    assert_eq!(d.hinglish.night_model(), None);
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("too slow to run on every chunk")
                && l.contains("best finished on a faster machine")),
        "{:#?}",
        d.verdict
    );
    // A model that runs during the recording needs no night pass of its own.
    p.measurements.retain(|m| m.model != "hinglish-q8_0");
    p.measurements.push(rolling(
        "hinglish-q8_0",
        Lang::Hi,
        DeviceConfig::Gpu { index: 0 },
        0.6,
        2.7,
        Some(2.0),
    ));
    let d = decide_with(&p, false);
    assert!(d.hinglish.ok);
    assert_eq!(d.hinglish.night_model(), None);
    assert!(
        d.verdict
            .iter()
            .all(|l| !l.contains("best finished on a faster machine")),
        "{:#?}",
        d.verdict
    );
}

#[test]
fn spoken_hindi_and_urdu_are_one_language_with_a_chosen_script() {
    assert_eq!(
        session_langs(true, true, HindiUrduScript::Devanagari),
        vec![Lang::En, Lang::Hi]
    );
    assert_eq!(
        session_langs(true, true, HindiUrduScript::Nastaliq),
        vec![Lang::En, Lang::Ur]
    );
    assert_eq!(
        session_langs(false, true, HindiUrduScript::Nastaliq),
        vec![Lang::Ur]
    );
}

#[test]
fn start_session_defaults_to_record_and_postprocess_and_explains_live() {
    // Low-baseline: live is disabled, with the reason.
    let c = cap(emu_low(), false);
    let plan = plan_session(&c, &[Lang::En, Lang::Hi]);
    assert_eq!(plan.default_mode, "record_postprocess");
    assert!(!plan.live.selectable);
    assert!(plan.live.reason.as_deref().unwrap().contains("real time"));
    assert_eq!(plan.devices, "Intel(R) UHD Graphics (Vulkan)");
    assert_eq!(plan.models.len(), 2);
    assert!(plan.ready_after_stop_min.unwrap() > 7.3 && plan.ready_after_stop_min.unwrap() < 7.4);

    // Tier G: selectable for English only; any Hindi or Urdu language makes it unavailable, with the reason.
    let g = cap(legion_native(), false);
    assert!(plan_session(&g, &[Lang::En]).live.selectable);
    assert!(!plan_session(&g, &[Lang::En, Lang::Hi]).live.selectable);
    assert!(!plan_session(&g, &[Lang::En, Lang::Ur]).live.selectable);
    assert!(
        plan_session(&g, &[Lang::Hi])
            .live
            .reason
            .as_deref()
            .unwrap()
            .contains("checked for accuracy")
    );
    let gv = cap(legion_native(), true);
    assert!(plan_session(&gv, &[Lang::En, Lang::Hi]).live.selectable);
    assert!(plan_session(&gv, &[Lang::En, Lang::Ur]).live.selectable);
    assert_eq!(
        plan_session(&g, &[Lang::En, Lang::Hi]).default_mode,
        "record_postprocess"
    );

    // C+ with validated near-live models: English and Hindi yes, Urdu no.
    let c = cap(strong_igpu_laptop(), true);
    assert!(plan_session(&c, &[Lang::En, Lang::Hi]).live.selectable);
    let ur = plan_session(&c, &[Lang::En, Lang::Ur]);
    assert!(!ur.live.selectable);
    assert!(ur.live.reason.as_deref().unwrap().contains("Urdu"));
}

#[test]
fn the_override_lifts_soft_reasons_but_never_the_hard_ones() {
    // Strong iGPU, near-live not validated: a soft reason. The override (confirmed) lifts it.
    let mut c = cap(strong_igpu_laptop(), false);
    assert!(!plan_session(&c, &[Lang::En]).live.selectable);
    set_override(
        &mut c,
        OverrideKind::AllowLiveAnyway,
        true,
        false,
        "2026-11-13T20:00:00+05:30",
    );
    assert!(
        !plan_session(&c, &[Lang::En]).live.selectable,
        "an unconfirmed switch changes nothing"
    );
    set_override(
        &mut c,
        OverrideKind::AllowLiveAnyway,
        true,
        true,
        "2026-11-13T20:01:00+05:30",
    );
    let plan = plan_session(&c, &[Lang::En]);
    assert!(plan.live.selectable && plan.live.via_override);

    // emu-low: small Hindi is slower than real time: hard, the override cannot lift it.
    let mut c = cap(emu_low(), true);
    set_override(&mut c, OverrideKind::AllowLiveAnyway, true, true, "t");
    let plan = plan_session(&c, &[Lang::Hi]);
    assert!(!plan.live.selectable);
    assert!(
        plan.live
            .reason
            .as_deref()
            .unwrap()
            .contains("slower than real time")
    );
    assert_eq!(c.overrides.len(), 1, "every change is logged");
}

#[test]
fn near_live_with_truncated_chunks_is_a_hard_block() {
    // strong iGPU laptop, validated, fast: but a truncated chunk in the Hindi near-live test.
    let mut p = strong_igpu_laptop();
    p.measurements
        .iter_mut()
        .find(|m| m.mode == Mode::NearLive && m.lang == Lang::Hi)
        .unwrap()
        .truncated_chunks = 2;
    let d = decide_with(&p, true);
    let hi = &d.languages[&Lang::Hi].live;
    assert!(!hi.offered);
    assert!(hi.hard_reasons[0].contains("lost speech"));
    assert!(d.languages[&Lang::En].live.offered);
}

#[test]
fn near_live_not_measured_for_want_of_a_validated_model_says_so_and_stays_off() {
    let mut p = emu_low();
    p.measurements.retain(|m| m.mode != Mode::NearLive);
    // No near-live model is validated: the check does not time one, and the reason says why.
    let d = decide_with(&p, false);
    for lang in [Lang::En, Lang::Hi] {
        let live = &d.languages[&lang].live;
        assert!(!live.offered, "{lang:?}");
        assert!(
            live.hard_reasons
                .iter()
                .any(|r| r.contains("Not measured: no validated near-live model")),
            "{lang:?}: {:?}",
            live.hard_reasons
        );
    }
    // With a validated model an unmeasured speed is still a hard reason, with the older wording.
    let v = decide_with(&p, true);
    assert!(
        v.languages[&Lang::En]
            .live
            .hard_reasons
            .iter()
            .any(|r| r.contains("has not been measured yet"))
    );
}

#[test]
fn a_profile_saved_before_the_skipped_list_still_loads_and_the_list_round_trips() {
    let c = cap(emu_low(), false);
    let mut v: serde_json::Value = serde_json::to_value(&c).unwrap();
    v.as_object_mut().unwrap().remove("skipped");
    let old = parse_capability(&v.to_string()).unwrap();
    assert!(old.profile.skipped.is_empty());
    let mut c2 = c.clone();
    c2.profile.skipped.push(Skipped {
        step: "hybrid test".into(),
        reason: "predicted".into(),
    });
    let text = serde_json::to_string(&c2).unwrap();
    assert_eq!(parse_capability(&text).unwrap().profile.skipped.len(), 1);
}

#[test]
fn capability_json_round_trips_and_refuses_another_schema() {
    let c = cap(legion_native(), false);
    let text = serde_json::to_string_pretty(&c).unwrap();
    assert_eq!(parse_capability(&text).unwrap(), c);
    let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
    v["schema_version"] = serde_json::json!(99);
    assert!(matches!(
        parse_capability(&v.to_string()),
        Err(CapabilityError::Schema(99))
    ));
    assert!(parse_capability("{").is_err());
}

#[test]
fn fingerprint_id_changes_with_the_hardware_and_lists_the_difference() {
    let a = fingerprint("Core i5", 16, None);
    let mut b = a.clone();
    assert_eq!(a.id(), b.id());
    assert!(a.differences(&b).is_empty());
    b.gpus[0].driver = Some("2.0".into());
    assert_ne!(a.id(), b.id());
    assert_eq!(b.differences(&a).len(), 1);
    assert!(b.differences(&a)[0].contains("graphics"));
    let e = fingerprint("Core i5", 16, Some("emu-low"));
    assert_ne!(a.id(), e.id());
    assert!(a.id().len() == 9 && a.id().contains('-'));
}

#[test]
fn model_fit_check_before_measuring() {
    let hw = hardware(4, 8, 16.0, 11.0, vec![]);
    let o = Options::default();
    assert!(model_fits(&hw, 1_081_140_203, &o));
    assert!(!model_fits(&hw, 8 * GIB, &o));
}

/// Writes (WB_UPDATE_FIXTURES=1) or checks the committed capability.json fixtures that the web UI's
/// mocked bridge and the Playwright tests load, so the UI shows what these rules decide.
#[test]
fn committed_fixtures_equal_the_decisions() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let all: Vec<(&str, Capability)> = vec![
        ("tier-g", cap(legion_native(), false)),
        ("tier-c-plus", cap(strong_igpu_laptop(), true)),
        ("tier-c-low", cap(emu_low(), false)),
        ("tier-c-mid-hybrid", cap(emu_mid_hybrid(), false)),
        ("tier-c-cpu-only", cap(cpu_only_weak(), false)),
        ("tier-r", cap(old_pc(), false)),
        ("slow-igpu", cap(slow_igpu(), false)),
        ("avx512-crash", cap(avx512_crash(), false)),
    ];
    let update = std::env::var("WB_UPDATE_FIXTURES").is_ok();
    if update {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, c) in all {
        let text = serde_json::to_string_pretty(&c).unwrap() + "\n";
        let path = dir.join(format!("{name}.json"));
        if update {
            std::fs::write(&path, &text).unwrap();
        } else {
            let on_disk = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                panic!(
                    "{} is missing: run with WB_UPDATE_FIXTURES=1",
                    path.display()
                )
            });
            assert_eq!(
                on_disk.replace("\r\n", "\n"),
                text,
                "{name}.json is out of date: run with WB_UPDATE_FIXTURES=1"
            );
        }
    }
}

#[test]
fn processing_while_recording_needs_a_passed_safety_run_for_the_chosen_setup() {
    // A complete check: allowed.
    let d = decide_with(&emu_low(), false);
    assert!(d.process_while_recording);
    assert!(!d.partial);

    // No safety run at all (skipped over budget, no input device): transcription waits for Stop and the
    // profile is partial.
    let mut p = emu_low();
    p.recorder_safety.clear();
    let d = decide_with(&p, false);
    assert!(!d.process_while_recording);
    assert!(d.partial);
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("safety test has not run"))
    );
    assert_eq!(d.config.as_ref().unwrap().recorder_safety_passed, None);
}

#[test]
fn hybrid_is_refused_when_two_engines_do_not_fit_in_memory() {
    // Both engines' peaks (1.38 + 1.49 GiB), the integrated GPU's memory (0.88 GiB) and the 3 GB reserve fit
    // in 11 GiB: hybrid chosen.
    let d = decide_with(&emu_mid_hybrid(), false);
    assert!(matches!(
        d.config.as_ref().unwrap().config,
        DeviceConfig::Hybrid { .. }
    ));

    // With 6 GiB free one engine fits (1.49 + 0.88 + 3) but two do not (1.38 + 1.49 + 0.88 + 3): the GPU alone.
    let mut p = emu_mid_hybrid();
    p.hardware.available_ram_bytes = 6 * 1_073_741_824;
    let d = decide_with(&p, false);
    assert!(
        !matches!(
            d.config.as_ref().unwrap().config,
            DeviceConfig::Hybrid { .. }
        ),
        "{:?}",
        d.config
    );
}

// ---- video memory of a discrete card (owner, 26 Nov 2026) ----

/// The Legion with its RTX card made `card_gib` large and the Hinglish f16 (4.01 GiB peak) or q8_0 (2.67 GiB peak) cell
/// measured on it, as in docs/evidence/phase-5/fit-and-fallback.txt.
fn legion_with_card(card_gib: f64, hinglish: Option<(&str, f64)>) -> Profile {
    let mut p = legion_native();
    for g in p.hardware.gpus.iter_mut().filter(|g| g.index == 1) {
        g.dedicated_bytes = gib(card_gib);
        g.local_bytes = gib(card_gib);
    }
    p.measurements.retain(|m| !m.model.starts_with("hinglish"));
    if let Some((model, peak_gib)) = hinglish {
        p.measurements.push(rolling(
            model,
            Lang::Hi,
            DeviceConfig::Gpu { index: 1 },
            0.19,
            5.0,
            Some(peak_gib),
        ));
    }
    p
}

#[test]
fn a_model_is_used_on_a_discrete_card_only_if_its_peak_plus_half_a_gigabyte_fits_the_card() {
    // A Hinglish cell that peaks at 4.01 GiB (the f16 file's peak; the file is no longer shipped, the memory rule is the
    // same for any model): with 0.5 GiB kept free it needs 4.51 GiB, so a 4 GB card cannot take it.
    for (card, f16_fits) in [(4.0, false), (6.0, true), (8.0, true)] {
        let d = decide_with(
            &legion_with_card(card, Some(("hinglish-q8_0", 4.01))),
            false,
        );
        assert_eq!(d.hinglish.ok, f16_fits, "{card} GB card: {:?}", d.hinglish);
        assert_eq!(
            d.hinglish.chosen_model(),
            f16_fits.then_some("hinglish-q8_0"),
            "{card} GB card"
        );
        let says_no_fit = d
            .verdict
            .iter()
            .any(|l| l.contains("hinglish-q8_0 does not fit this card's memory"));
        assert_eq!(says_no_fit, !f16_fits, "{card} GB card: {:#?}", d.verdict);
    }
    // q8_0 peaks at 2.67 GiB: it fits all three (with no live model beside it).
    for card in [4.0, 6.0, 8.0] {
        let mut p = legion_with_card(card, Some(("hinglish-q8_0", 2.67)));
        p.measurements.retain(|m| !m.live_standin);
        let d = decide_with(&p, false);
        assert_eq!(
            d.hinglish.chosen_model(),
            Some("hinglish-q8_0"),
            "{card} GB card"
        );
        assert!(
            !d.verdict
                .iter()
                .any(|l| l.contains("does not fit this card's memory")),
            "{card} GB card: {:#?}",
            d.verdict
        );
    }
}

/// A pair cell on the Legion's card (index 1): the Hinglish model with `small-q5_0` in front of it, as the system check
/// measures it (a batch decode with detection on; memory is the two models' cells added up).
fn pair_cell(model: &str, rtf: f64, local_gib: f64) -> Measurement {
    let mut m = rolling(
        model,
        Lang::Hi,
        DeviceConfig::Gpu { index: 1 },
        rtf,
        6.0,
        Some(local_gib),
    );
    m.detector = Some("small-q5_0".to_string());
    m
}

#[test]
fn a_pair_that_meets_the_bar_and_fits_the_card_is_chosen_over_the_model_alone() {
    // 8 GB card: q8_0 alone (0.19) and q8_0 with the small detector (0.25, 3.18 GiB for both) were measured.
    let mut p = legion_with_card(8.0, Some(("hinglish-q8_0", 2.67)));
    p.measurements.push(pair_cell("hinglish-q8_0", 0.25, 3.18));
    let d = decide_with(&p, false);
    assert!(d.hinglish.ok, "{:?}", d.hinglish);
    assert_eq!(d.hinglish.chosen_model(), Some("hinglish-q8_0"));
    assert_eq!(d.hinglish.chosen_detector(), Some("small-q5_0"));
    assert_eq!(d.hinglish.rtf, Some(0.25));
    assert!(
        d.verdict.iter().any(|l| l.contains(
            "hinglish-q8_0 for the Hindi chunks, with the small model small-q5_0 in front of it"
        )),
        "{:#?}",
        d.verdict
    );
    // The pair is slower than the bar: the model alone, which meets it, is chosen.
    let mut p = legion_with_card(8.0, Some(("hinglish-q8_0", 2.67)));
    p.measurements.push(pair_cell("hinglish-q8_0", 5.0, 3.18));
    let d = decide_with(&p, false);
    assert_eq!(d.hinglish.chosen_model(), Some("hinglish-q8_0"));
    assert_eq!(d.hinglish.chosen_detector(), None);
}

#[test]
fn a_pair_that_does_not_fit_the_card_is_not_chosen() {
    // q8_0 with the detector peaks at 3.18 GiB: with the headroom it needs 3.68 GiB, so a 3 GB card cannot take it.
    let mut p = legion_with_card(3.0, None);
    p.measurements.push(pair_cell("hinglish-q8_0", 0.25, 3.18));
    let d = decide_with(&p, false);
    assert_eq!(d.hinglish.chosen_model(), None, "{:?}", d.hinglish);
    assert!(!d.hinglish.ok);
}

#[test]
fn the_f16_model_is_gone_it_is_never_chosen_even_if_its_cell_is_in_the_file() {
    // The STALE case (owner, 27 Nov 2026): a capability file from the earlier build can hold f16 cells, alone or with the
    // small detector. The file is not in the manifest any more, so no cell of it is used: only ids that the system check
    // measures and the manifest lists (`HINGLISH_MODELS`) are, and the pairs `HINGLISH_PAIRS` lists. The earlier code took
    // any id that started with "hinglish".
    assert_eq!(HINGLISH_PAIRS, &[("hinglish-q8_0", "small-q5_0")]);
    assert_eq!(HINGLISH_MODELS, &["hinglish-q5_0", "hinglish-q8_0"]);
    let mut p = legion_with_card(8.0, Some(("hinglish-f16", 4.01)));
    p.measurements.push(pair_cell("hinglish-f16", 0.25, 4.5));
    let d = decide_with(&p, false);
    assert_eq!(d.hinglish.chosen_model(), None, "{:?}", d.hinglish);
    assert_eq!(d.hinglish.night_model(), None, "{:?}", d.hinglish);
    let mut p = legion_with_card(8.0, None);
    p.measurements.push(pair_cell("hinglish-f16", 0.25, 4.5));
    assert_eq!(
        decide_with(&p, false).hinglish.chosen_model(),
        None,
        "an f16 pair alone gives nothing"
    );
    // An id that merely starts with "hinglish" is not a model of the manifest either.
    let p = legion_with_card(8.0, Some(("hinglish-new", 2.0)));
    assert_eq!(decide_with(&p, false).hinglish.chosen_model(), None);
}

#[test]
fn a_four_gigabyte_card_gets_the_q8_0_pair_and_live_text_gives_way() {
    // q8_0 with the small detector peaked at 3.18 GiB (3255 MiB, RTX 4070): 3.68 GiB with the headroom fits 4 GB.
    for (card, live_offered) in [(4.0, false), (6.0, true), (8.0, true)] {
        let mut p = legion_with_card(card, None);
        p.measurements.push(pair_cell("hinglish-q8_0", 0.25, 3.18));
        let d = decide_with(&p, false);
        assert!(d.hinglish.ok, "{card} GB: {:?}", d.hinglish);
        assert_eq!(
            d.hinglish.chosen_model(),
            Some("hinglish-q8_0"),
            "{card} GB"
        );
        assert_eq!(
            d.hinglish.chosen_detector(),
            Some("small-q5_0"),
            "{card} GB"
        );
        let live = &d.languages[&Lang::En].live;
        assert_eq!(live.offered, live_offered, "{card} GB: {live:?}");
        let says_beside = live.hard_reasons.iter().any(|r| {
            r.contains("does not fit this card's memory beside hinglish-q8_0 with small-q5_0")
        });
        assert_eq!(says_beside, !live_offered, "{card} GB: {live:?}");
    }
}

#[test]
fn only_a_pair_the_evaluation_accepted_is_ever_chosen() {
    assert!(
        HINGLISH_PAIRS
            .iter()
            .all(|(h, d)| h.starts_with("hinglish") && *d == "small-q5_0")
    );
    let mut p = legion_with_card(8.0, None);
    let mut m = pair_cell("hinglish-q8_0", 0.25, 3.18);
    m.detector = Some("tiny".to_string());
    p.measurements.push(m);
    let d = decide_with(&p, false);
    assert_eq!(d.hinglish.chosen_model(), None, "{:?}", d.hinglish);
}

#[test]
fn a_measured_model_that_does_not_fit_the_card_is_not_used_for_the_rolling_pass_either() {
    // Medium peaks at 2.26 GiB in the fixture; on a 2.5 GiB card it leaves less than 0.5 GiB: it must not be chosen.
    let d = decide_with(&legion_with_card(2.5, None), false);
    for lang in [Lang::En, Lang::Hi] {
        if let Some(r) = d.languages[&lang].rolling.as_ref() {
            assert_ne!(r.model, "medium", "{lang:?}");
        }
    }
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("medium does not fit this card's memory")),
        "{:#?}",
        d.verdict
    );
}

#[test]
fn two_models_that_do_not_fit_the_card_together_become_one_model_for_every_language() {
    // Medium (2.26 GiB) beside turbo (0.9 GiB) is 3.16 GiB; with 0.5 GiB free a 3.5 GiB card cannot hold both.
    let p = legion_with_card(3.5, None);
    let d = decide_with(&p, false);
    let models: std::collections::BTreeSet<&str> = Lang::ALL
        .iter()
        .filter_map(|l| d.languages[l].rolling.as_ref().map(|r| r.model.as_str()))
        .collect();
    assert_eq!(models.len(), 1, "{models:?}");
    assert!(
        d.verdict
            .iter()
            .any(|l| l.contains("together do not fit this card's memory")
                && l.contains("decodes every language")),
        "{:#?}",
        d.verdict
    );
    // On the 8 GB card both stay: medium for English, turbo for Hindi.
    let d8 = decide_with(&legion_with_card(8.0, None), false);
    assert_eq!(rolling_of(&d8, Lang::En).model, "medium");
    assert_eq!(rolling_of(&d8, Lang::Hi).model, "large-v3-turbo-q5_0");
    assert!(!d8.verdict.iter().any(|l| l.contains("this card's memory")));
}

#[test]
fn before_measuring_a_model_is_tried_on_a_card_only_if_its_file_times_1_15_plus_half_a_gigabyte_fits()
 {
    let o = Options::default();
    let f16_file = 3_095_033_483u64; // the Hinglish f16 file
    let q8_file = 1_656_538_283u64;
    for (card, f16_fits) in [(4.0, false), (6.0, true), (8.0, true)] {
        let p = legion_with_card(card, None);
        let gpu = DeviceConfig::Gpu { index: 1 };
        assert_eq!(
            model_fits_card(&p, gpu, f16_file, &o),
            f16_fits,
            "{card} GB card"
        );
        assert_eq!(
            model_fits_card(&p, DeviceConfig::Hybrid { gpu: 1 }, f16_file, &o),
            f16_fits,
            "{card} GB card, hybrid"
        );
        assert!(model_fits_card(&p, gpu, q8_file, &o), "{card} GB card");
        // The processor and an integrated adapter have no card to fill.
        assert!(model_fits_card(&p, DeviceConfig::Cpu, f16_file, &o));
        assert!(model_fits_card(
            &p,
            DeviceConfig::Gpu { index: 0 },
            f16_file,
            &o
        ));
    }
    assert_eq!(
        estimated_vram_bytes(1_000_000_000, &o),
        1_150_000_000 + (1u64 << 29)
    );
    assert!(vram_fits(gib(4.0), gib(3.5), &o));
    assert!(!vram_fits(gib(4.0), gib(3.6), &o));
}

#[test]
fn a_card_whose_memory_could_not_be_read_is_not_judged() {
    let mut p = legion_with_card(8.0, Some(("hinglish-q8_0", 4.01)));
    for g in p.hardware.gpus.iter_mut().filter(|g| g.index == 1) {
        g.dedicated_bytes = 0;
    }
    assert!(model_fits_card(
        &p,
        DeviceConfig::Gpu { index: 1 },
        10_000_000_000,
        &Options::default()
    ));
    assert!(decide_with(&p, false).hinglish.ok);
}

/// Phase 5, item 6 (owner, 27 Nov 2026): the bench decodes one language with no detection, a session with several languages
/// detects every chunk and runs slower. A pick must meet the bar WITH the allowance: a model that passes on the bench figure
/// and fails with the allowance is not a pick that meets the bar, and the verdict says why. The measured figure stays.
#[test]
fn a_pick_must_meet_the_bar_with_the_detection_allowance_not_only_on_the_bench_figure() {
    let p = avx512_crash(); // English medium 0.60, Hindi turbo 0.70, Urdu medium 0.80; bar 1.0 (not emulated)
    let none = decide(&p, &rankings(false), &Options::default());
    let o = Options {
        rolling_detection_allowance: 1.5,
        ..Options::default()
    };
    let with = decide(&p, &rankings(false), &o);
    for lang in [Lang::En, Lang::Hi, Lang::Ur] {
        let (a, b) = (rolling_of(&none, lang), rolling_of(&with, lang));
        assert!(a.meets_bar, "{lang:?} meets the bench bar");
        assert_eq!(
            (&a.model, a.rtf),
            (&b.model, b.rtf),
            "{lang:?}: the measured figure stays"
        );
        assert_eq!(a.rtf_with_detection, None, "{lang:?}");
        assert!(
            (b.rtf_with_detection.unwrap() - a.rtf * 1.5).abs() < 1e-9,
            "{lang:?}"
        );
    }
    // 0.60 x 1.5 = 0.90 still meets the bar; 0.70 x 1.5 = 1.05 and 0.80 x 1.5 = 1.2 do not.
    assert!(rolling_of(&with, Lang::En).meets_bar);
    assert!(!rolling_of(&with, Lang::Hi).meets_bar);
    assert!(!rolling_of(&with, Lang::Ur).meets_bar);
    assert!(rolling_of(&with, Lang::Hi).lags && !rolling_of(&none, Lang::Hi).lags);
    assert_eq!(
        (
            rolling_of(&none, Lang::Hi).k_max,
            rolling_of(&with, Lang::Hi).k_max
        ),
        (1, 0)
    );
    let says = |d: &Decision, lang: &str| {
        d.verdict.iter().any(|v| {
            v.starts_with(lang)
                && v.contains("meets the bar on one language")
                && v.contains("detects the language of every chunk")
        })
    };
    assert!(
        says(&with, "Hindi") && says(&with, "Urdu"),
        "{:?}",
        with.verdict
    );
    assert!(!says(&with, "English"));
    assert!(
        !none
            .verdict
            .iter()
            .any(|v| v.contains("detects the language of every chunk"))
    );
}
