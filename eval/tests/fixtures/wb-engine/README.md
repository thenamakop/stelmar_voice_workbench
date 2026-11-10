# wb-engine event fixtures

Replay input for the `wb-engine` adapter of `eval/run.py` until Phase 2's engine is merged: JSONL files of
engine events in the shape from `docs/prompts/phase-2-engines-and-models.md` (`segment` with `start_ms`,
`end_ms`, `text`, `lang`, `lang_prob`, `mixed`, `model`; then `done`).

`synthetic-hi/` is hand-written: the sentences were composed for these tests and are not reference
transcripts or model output for any dataset clip (FLEURS and CoSHE-Eval text is never committed; see
`eval/README.md`). They only exercise the event parser. To replay real engine output for a dataset, record
fixtures locally with `eval/run.py record-fixture --setup <setup> --dataset <set> --limit <n> --out <folder>`
and keep the folder out of git.
