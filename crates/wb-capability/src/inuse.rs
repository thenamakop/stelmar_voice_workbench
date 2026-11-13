//! Which saved profile is in use (owner's decisions 3 and 4 of 14 Nov 2026).
//!
//! The app keeps up to two files: `capability.json` (the profile in use) and `capability.partial.json` (a
//! partial or cancelled check kept apart so that it never replaces a complete profile). After the computer's
//! fingerprint changed, neither profile describes it: the partial one that was measured on this computer is
//! used if there is one, and either way with safe defaults (record + post-process only: no live text, no
//! processing while recording) until a new check has run and been saved as the profile in use.
//!
//! This module is pure: the app loads the files and the current fingerprint and asks here.

use serde::{Deserialize, Serialize};

use crate::{Capability, Fingerprint};

/// Which file a profile lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedFile {
    /// `capability.json`.
    Main,
    /// `capability.partial.json`.
    Partial,
}

/// The profile the rest of the app reads, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct InUse {
    pub cap: Capability,
    pub file: SavedFile,
    /// What differs between the computer and the profile that was in use; empty when it still fits. When not
    /// empty, `cap` carries safe defaults.
    pub changes: Vec<String>,
}

fn changes_for(cap: &Capability, now: Option<&Fingerprint>) -> Vec<String> {
    now.map(|n| n.differences(&cap.profile.fingerprint))
        .unwrap_or_default()
}

/// Where a new check's profile goes. A partial or cancelled check never replaces a complete profile in
/// `capability.json`: it is kept in `capability.partial.json`. With no complete profile saved (none at all, or
/// only a partial one) it becomes the profile in use.
pub fn route_new_check(existing_main: Option<&Capability>, new: &Capability) -> SavedFile {
    let protects = existing_main.is_some_and(|m| !m.profile.check.partial);
    if new.profile.check.partial && protects {
        SavedFile::Partial
    } else {
        SavedFile::Main
    }
}

/// The profile in use. `now` is this computer's fingerprint, `None` when it could not be taken yet (the
/// profile is then taken as it is).
pub fn choose_in_use(
    main: Option<Capability>,
    partial: Option<Capability>,
    now: Option<&Fingerprint>,
) -> Option<InUse> {
    match (main, partial) {
        (Some(m), p) => {
            let changes = changes_for(&m, now);
            if changes.is_empty() {
                return Some(InUse {
                    cap: m,
                    file: SavedFile::Main,
                    changes,
                });
            }
            // The complete profile is of another machine state. The partial check, if it was measured on this
            // computer, is the nearest description; both get safe defaults.
            if let Some(p) = p
                && changes_for(&p, now).is_empty()
            {
                let why = safe_note(&changes, true);
                return Some(InUse {
                    cap: safe_defaults(&p, &why),
                    file: SavedFile::Partial,
                    changes,
                });
            }
            let why = safe_note(&changes, false);
            Some(InUse {
                cap: safe_defaults(&m, &why),
                file: SavedFile::Main,
                changes,
            })
        }
        (None, Some(p)) => {
            let changes = changes_for(&p, now);
            let cap = if changes.is_empty() {
                p
            } else {
                safe_defaults(&p, &safe_note(&changes, false))
            };
            Some(InUse {
                cap,
                file: SavedFile::Partial,
                changes,
            })
        }
        (None, None) => None,
    }
}

/// A profile that cannot be compared with this computer yet (the fingerprint could not be taken: the engines
/// have not been probed, or the model list could not be read) is not known to fit it. Where a decision is
/// made on it (starting live text, changing an override) it carries safe defaults too; the System screen
/// keeps showing it as it is until the fingerprint is known.
pub fn unidentified(used: InUse) -> InUse {
    if !used.changes.is_empty() {
        return used;
    }
    let why = "This computer has not been identified yet (its engines have not been probed). Safe defaults apply: recording and post-processing only, until the check of this computer is known to fit.".to_string();
    InUse {
        cap: safe_defaults(&used.cap, &why),
        file: used.file,
        changes: vec![why],
    }
}

fn safe_note(changes: &[String], partial: bool) -> String {
    let what = changes.first().map_or("something", String::as_str);
    if partial {
        format!(
            "This computer has changed since the last complete check ({what}). The partial check made on this computer is used, with safe defaults: recording and post-processing only, until a new check has run."
        )
    } else {
        format!(
            "This computer has changed since the check ({what}). Safe defaults apply: recording and post-processing only, until a new check has run."
        )
    }
}

/// The profile with safe defaults: live text off for every language (a reason no override can lift), no
/// processing while recording, the operator's earlier overrides not applied (they were for another machine
/// state), the verdict marked provisional and saying why. Recording and post-processing stay as measured.
pub fn safe_defaults(cap: &Capability, why: &str) -> Capability {
    let mut c = cap.clone();
    c.overrides.clear();
    let d = &mut c.decision;
    d.process_while_recording = false;
    d.provisional = true;
    for l in d.languages.values_mut() {
        l.live.offered = false;
        l.live.kind = None;
        l.live.model = None;
        l.live.hard_reasons.insert(0, why.to_string());
    }
    d.verdict.insert(0, why.to_string());
    c
}
