//! Which saved profile is in use (owner's decisions 3 and 4, 14 Nov 2026).

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use common::*;
use wb_capability::*;

fn complete(p: Profile) -> Capability {
    let mut c = cap(p, false);
    c.profile.check.partial = false;
    c.decision.partial = false;
    c
}

fn partial(mut c: Capability) -> Capability {
    c.profile.check.partial = true;
    c.decision.partial = true;
    c
}

fn changed(fp: &Fingerprint) -> Fingerprint {
    let mut f = fp.clone();
    f.os = format!("{} (updated)", f.os);
    f
}

#[test]
fn a_partial_check_never_replaces_a_complete_profile() {
    let main = complete(legion_native());
    let new_partial = partial(complete(legion_native()));
    assert_eq!(
        route_new_check(Some(&main), &new_partial),
        SavedFile::Partial
    );
    // A complete check replaces the profile in use.
    assert_eq!(
        route_new_check(Some(&main), &complete(legion_native())),
        SavedFile::Main
    );
    // With no complete profile saved, a partial one is what there is.
    assert_eq!(route_new_check(None, &new_partial), SavedFile::Main);
    assert_eq!(
        route_new_check(Some(&new_partial), &new_partial),
        SavedFile::Main
    );
}

#[test]
fn a_profile_that_still_fits_is_used_as_it_is_and_the_partial_one_is_only_listed() {
    let main = complete(legion_native());
    let now = main.profile.fingerprint.clone();
    let p = partial(main.clone());
    let used = choose_in_use(Some(main.clone()), Some(p), Some(&now)).unwrap();
    assert_eq!(used.file, SavedFile::Main);
    assert!(used.changes.is_empty());
    assert_eq!(used.cap, main);
    assert!(
        used.cap.decision.languages[&Lang::En].live.offered,
        "English live stays offered on tier G"
    );
}

#[test]
fn after_a_fingerprint_change_live_and_processing_while_recording_are_refused_but_post_processing_stays()
 {
    let main = complete(legion_native());
    assert!(main.decision.process_while_recording);
    let now = changed(&main.profile.fingerprint);
    let used = choose_in_use(Some(main.clone()), None, Some(&now)).unwrap();
    assert_eq!(used.file, SavedFile::Main);
    assert!(!used.changes.is_empty());
    let d = &used.cap.decision;
    assert!(!d.process_while_recording);
    assert!(
        d.config.is_some(),
        "recording and post-processing stay available"
    );
    assert!(d.languages.values().all(|l| !l.live.offered));
    assert!(d.verdict[0].contains("changed"));
    // No override lifts it, not even "allow live captions anyway".
    let mut with_override = used.cap.clone();
    with_override.overrides.push(OverrideRecord {
        kind: OverrideKind::AllowLiveAnyway,
        enabled: true,
        at: "2026-11-14T10:00:00+05:30".into(),
        confirmed: true,
    });
    let plan = plan_session(&with_override, &[Lang::En]);
    assert!(!plan.live.selectable);
    assert!(plan.live.reason.unwrap().contains("changed"));
    assert!(!plan.record_only, "post-processing stays");
}

#[test]
fn after_a_fingerprint_change_the_partial_check_of_this_computer_is_used_with_safe_defaults() {
    let main = complete(legion_native());
    let now = changed(&main.profile.fingerprint);
    let mut p = partial(complete(legion_native()));
    p.profile.fingerprint = now.clone();
    let used = choose_in_use(Some(main.clone()), Some(p.clone()), Some(&now)).unwrap();
    assert_eq!(used.file, SavedFile::Partial);
    assert!(!used.cap.decision.process_while_recording);
    assert!(
        used.cap
            .decision
            .languages
            .values()
            .all(|l| !l.live.offered)
    );
    assert!(used.cap.decision.verdict[0].contains("partial check"));
    assert!(used.cap.profile.check.partial);

    // A partial check made on some other state of the machine is not used.
    let mut other = p;
    other.profile.fingerprint = changed(&now);
    let used = choose_in_use(Some(main), Some(other), Some(&now)).unwrap();
    assert_eq!(used.file, SavedFile::Main);
}

#[test]
fn a_profile_that_cannot_be_compared_yet_carries_safe_defaults_where_decisions_are_made() {
    let main = complete(legion_native());
    let used = wb_capability::unidentified(choose_in_use(Some(main), None, None).unwrap());
    assert!(!used.changes.is_empty());
    assert!(!used.cap.decision.process_while_recording);
    assert!(
        used.cap
            .decision
            .languages
            .values()
            .all(|l| !l.live.offered)
    );
    // One that already has changes is left as it is (safe defaults are not applied twice).
    let again = wb_capability::unidentified(used.clone());
    assert_eq!(again, used);
}

#[test]
fn a_fingerprint_that_could_not_be_taken_leaves_the_profile_as_it_is() {
    let main = complete(legion_native());
    let used = choose_in_use(Some(main.clone()), None, None).unwrap();
    assert!(used.changes.is_empty());
    assert_eq!(used.cap, main);
    assert!(choose_in_use(None, None, None).is_none());
}

#[test]
fn only_a_partial_profile_on_a_changed_machine_gets_safe_defaults_too() {
    let p = partial(complete(legion_native()));
    let now = changed(&p.profile.fingerprint);
    let used = choose_in_use(None, Some(p), Some(&now)).unwrap();
    assert_eq!(used.file, SavedFile::Partial);
    assert!(!used.cap.decision.process_while_recording);
}
