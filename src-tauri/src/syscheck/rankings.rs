//! The system check's model lists, from `models/manifest.json` (`provisional_rank`, Phase 3).
//!
//! The check tries each language's models in rank order and stops at the first that meets the speed
//! bar. Every place is provisional until Phase 5 validates models; a place marked
//! `quality_unverified` has no local, independent WER behind it, and the screens say so.

use std::collections::BTreeMap;

use wb_capability::{Lang, Ranked, Rankings};
use wb_models::Manifest;

/// Role names as the manifest writes them.
pub const ROLE_ROLLING: &str = "rolling";
pub const ROLE_NIGHT: &str = "night";
pub const ROLE_NEAR_LIVE: &str = "near-live";
pub const ROLE_LIVE: &str = "live";

/// The ranked lists of every role and language in the manifest.
pub fn from_manifest(manifest: &Manifest) -> Rankings {
    let mut rolling: BTreeMap<Lang, Vec<(u32, Ranked)>> = BTreeMap::new();
    let mut night = rolling.clone();
    let mut near_live = rolling.clone();
    let mut live = rolling.clone();
    let mut any = false;
    for entry in &manifest.models {
        for place in &entry.provisional_rank {
            let Some(lang) = Lang::parse(&place.lang) else {
                continue;
            };
            let target = match place.role.as_str() {
                ROLE_ROLLING => &mut rolling,
                ROLE_NIGHT => &mut night,
                ROLE_NEAR_LIVE => &mut near_live,
                ROLE_LIVE => &mut live,
                _ => continue,
            };
            any = true;
            target.entry(lang).or_default().push((
                place.rank,
                Ranked {
                    model: entry.id.clone(),
                    bytes: entry.bytes,
                    quality_unverified: place.quality_unverified,
                    validated: entry.validated_roles.iter().any(|r| r == &place.role),
                },
            ));
        }
    }
    let finish = |m: BTreeMap<Lang, Vec<(u32, Ranked)>>| {
        m.into_iter()
            .map(|(lang, mut list)| {
                list.sort_by_key(|(rank, r)| (*rank, r.model.clone()));
                (lang, list.into_iter().map(|(_, r)| r).collect::<Vec<_>>())
            })
            .collect::<BTreeMap<_, _>>()
    };
    Rankings {
        rolling: finish(rolling),
        night: finish(night),
        near_live: finish(near_live),
        live: finish(live),
        provisional: any,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use wb_models::{Entry, RankPlace, Source};

    fn entry(id: &str, places: &[(&str, &str, u32, bool)], validated: &[&str]) -> Entry {
        Entry {
            id: id.into(),
            file: format!("{id}.bin"),
            sha256: String::new(),
            bytes: 1000,
            kind: "whisper".into(),
            base: String::new(),
            quant: String::new(),
            languages: vec![],
            candidate_roles: vec![],
            validated_roles: validated.iter().map(|s| (*s).into()).collect(),
            provisional_rank: places
                .iter()
                .map(|(role, lang, rank, q)| RankPlace {
                    role: (*role).into(),
                    lang: (*lang).into(),
                    rank: *rank,
                    quality_unverified: *q,
                })
                .collect(),
            licence: String::new(),
            source: Source::default(),
            conversion: None,
        }
    }

    #[test]
    fn lists_follow_the_rank_and_keep_the_marks() {
        let m = Manifest {
            version: 2,
            models: vec![
                entry("b", &[("rolling", "hi", 2, true)], &[]),
                entry(
                    "a",
                    &[("rolling", "hi", 1, false), ("near-live", "en", 1, true)],
                    &["near-live"],
                ),
            ],
        };
        let r = from_manifest(&m);
        let hi: Vec<_> = r.rolling[&Lang::Hi]
            .iter()
            .map(|x| x.model.as_str())
            .collect();
        assert_eq!(hi, ["a", "b"]);
        assert!(r.rolling[&Lang::Hi][1].quality_unverified);
        assert!(r.near_live[&Lang::En][0].validated);
        assert!(!r.rolling[&Lang::Hi][0].validated);
        assert!(r.provisional);
    }

    #[test]
    fn a_manifest_without_ranks_is_not_provisional() {
        let r = from_manifest(&Manifest {
            version: 2,
            models: vec![],
        });
        assert!(!r.provisional && r.rolling.is_empty());
    }
}
