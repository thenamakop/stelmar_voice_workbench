//! What the Start Session dialog shows, from the saved capability: which mode is selectable for the
//! ticked languages, where post-processing will run and how long until the transcript is ready.

use serde::{Deserialize, Serialize};

use crate::rules::ready_after_stop_min;
use crate::types::*;

/// The output script of spoken Hindi/Urdu, chosen by the organiser (GATES, 13 Nov 2026). Devanagari
/// is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HindiUrduScript {
    Devanagari,
    Nastaliq,
}

impl HindiUrduScript {
    /// The decoding language the script selects: Devanagari -> `hi`, Nastaliq -> `ur`.
    pub fn lang(self) -> Lang {
        match self {
            HindiUrduScript::Devanagari => Lang::Hi,
            HindiUrduScript::Nastaliq => Lang::Ur,
        }
    }
}

/// The session's languages: English (if ticked) and the Hindi/Urdu language in the chosen script.
pub fn session_langs(english: bool, hindi_urdu: bool, script: HindiUrduScript) -> Vec<Lang> {
    let mut v = Vec::new();
    if english {
        v.push(Lang::En);
    }
    if hindi_urdu {
        v.push(script.lang());
    }
    v
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveState {
    pub selectable: bool,
    /// Plain-words reason shown on the disabled option.
    pub reason: Option<String>,
    /// Selectable only because of the "Allow live captions anyway" switch.
    pub via_override: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionModel {
    pub lang: Lang,
    pub model: String,
    pub quality_unverified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionPlan {
    /// Record + post-process: selected by default on every machine.
    pub default_mode: String,
    pub record_only: bool,
    pub record_only_reason: Option<String>,
    pub live: LiveState,
    pub devices: String,
    pub models: Vec<SessionModel>,
    pub ready_after_stop_min: Option<f64>,
    pub lags: bool,
    pub provisional: bool,
    pub partial: bool,
}

pub fn override_enabled(overrides: &[OverrideRecord], kind: OverrideKind) -> bool {
    overrides
        .iter()
        .rev()
        .find(|o| o.kind == kind)
        .is_some_and(|o| o.enabled && o.confirmed)
}

/// Is live text selectable for this language, counting the operator's overrides? The hard reasons
/// (slower than real time, lost speech) cannot be overridden.
pub fn live_state_for(cap: &Capability, lang: Lang) -> LiveState {
    let Some(ld) = cap.decision.languages.get(&lang) else {
        return LiveState {
            selectable: false,
            reason: Some(format!("{} has not been measured.", lang.name())),
            via_override: false,
        };
    };
    if ld.live.offered {
        return LiveState {
            selectable: true,
            reason: None,
            via_override: false,
        };
    }
    if !ld.live.hard_reasons.is_empty() {
        return LiveState {
            selectable: false,
            reason: ld.live.hard_reasons.first().cloned(),
            via_override: false,
        };
    }
    let can_override = override_enabled(&cap.overrides, OverrideKind::AllowLiveAnyway)
        && cap.decision.config.is_some()
        && ld.rolling.is_some();
    if can_override {
        return LiveState {
            selectable: true,
            reason: None,
            via_override: true,
        };
    }
    LiveState {
        selectable: false,
        reason: ld.live.soft_reasons.first().cloned(),
        via_override: false,
    }
}

/// The dialog's view of the profile for the ticked languages.
pub fn plan_session(cap: &Capability, langs: &[Lang]) -> SessionPlan {
    let d = &cap.decision;
    let mut models = Vec::new();
    let mut ready: Option<f64> = None;
    let mut lags = false;
    for lang in langs {
        if let Some(rp) = d.languages.get(lang).and_then(|l| l.rolling.as_ref()) {
            models.push(SessionModel {
                lang: *lang,
                model: rp.model.clone(),
                quality_unverified: rp.quality_unverified,
            });
            ready = Some(ready.map_or(rp.ready_after_stop_min, |r: f64| {
                r.max(rp.ready_after_stop_min)
            }));
            lags |= rp.lags;
        }
    }
    // The ready time follows the slowest ticked language's model; recompute from the RTF so a
    // session of one language is not held back by another.
    if ready.is_none() && d.config.is_some() {
        ready = langs
            .iter()
            .filter_map(|l| d.languages.get(l).and_then(|x| x.rolling.as_ref()))
            .map(|rp| ready_after_stop_min(rp.rtf))
            .reduce(f64::max);
    }

    let live = if langs.is_empty() {
        LiveState {
            selectable: false,
            reason: Some("Tick at least one language.".into()),
            via_override: false,
        }
    } else if d.config.is_none() {
        LiveState {
            selectable: false,
            reason: d
                .record_only_reason
                .clone()
                .or_else(|| Some("This machine records only.".into())),
            via_override: false,
        }
    } else {
        let states: Vec<LiveState> = langs.iter().map(|l| live_state_for(cap, *l)).collect();
        match states.iter().find(|s| !s.selectable) {
            Some(blocked) => LiveState {
                selectable: false,
                reason: blocked.reason.clone(),
                via_override: false,
            },
            None => LiveState {
                selectable: true,
                reason: None,
                via_override: states.iter().any(|s| s.via_override),
            },
        }
    };

    SessionPlan {
        default_mode: "record_postprocess".to_string(),
        record_only: d.config.is_none(),
        record_only_reason: d.record_only_reason.clone(),
        live,
        devices: d
            .config
            .as_ref()
            .map_or_else(|| "none".to_string(), |c| c.label.clone()),
        models,
        ready_after_stop_min: ready,
        lags,
        provisional: d.provisional,
        partial: d.partial,
    }
}

/// Adds or replaces an override record (the log keeps every change).
pub fn set_override(
    cap: &mut Capability,
    kind: OverrideKind,
    enabled: bool,
    confirmed: bool,
    at: &str,
) {
    cap.overrides.push(OverrideRecord {
        kind,
        enabled,
        at: at.to_string(),
        confirmed,
    });
}
