//! `capability.json` and the rules behind it (docs/PLAN.md section 3, Phase 3 plan of 13 Nov 2026).
//!
//! This crate is pure logic: the app measures (probe, `wb-engine bench`), builds a [`Profile`], and
//! [`decide`] turns the measurements into a [`Decision`]: tier, device configuration, rolling and night
//! model per language, live availability, and the time until a transcript is ready. Nothing here
//! reads a file, runs a process or knows a model's speed in advance.

pub mod fingerprint;
pub mod rules;
pub mod session;
pub mod types;

pub use fingerprint::{Fingerprint, GpuPrint};
pub use rules::{bars, decide, memory_ok, model_fits, ready_after_stop_min};
pub use session::{
    HindiUrduScript, LiveState, SessionModel, SessionPlan, live_state_for, override_applies,
    override_enabled, plan_session, session_langs, set_override,
};
pub use types::*;

/// Reads a `capability.json` text. A file with another schema version is refused (the app then
/// re-runs the check instead of guessing).
pub fn parse_capability(text: &str) -> Result<Capability, CapabilityError> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(CapabilityError::Json)?;
    let version = value
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if version != u64::from(SCHEMA_VERSION) {
        return Err(CapabilityError::Schema(version));
    }
    serde_json::from_value(value).map_err(CapabilityError::Json)
}

#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    #[error("capability.json is not valid: {0}")]
    Json(serde_json::Error),
    #[error("capability.json has schema version {0}, this app reads version {SCHEMA_VERSION}")]
    Schema(u64),
}

/// Builds a full capability from a profile.
pub fn assemble(
    profile: Profile,
    rankings: &Rankings,
    options: &Options,
    overrides: Vec<OverrideRecord>,
) -> Capability {
    let decision = decide(&profile, rankings, options);
    Capability {
        profile,
        decision,
        overrides,
    }
}
