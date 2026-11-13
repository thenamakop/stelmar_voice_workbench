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
        "hinglish-q5_0",
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
