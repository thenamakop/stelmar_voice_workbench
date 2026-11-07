# Phase 7: Live mode, near-live text and captions

## Context
The record + post-process path is complete. Now add live text as an opt-in: it is off by default on every machine, GPU or not, and starts only when the operator picks it in the Start Session dialog on a machine the capability profile has cleared for all of the session's languages (decisions D4, D5 and D9; AGENTS.md non-negotiable 7). Streaming runs on GPU machines (tier G) and near-live utterance mode on fast CPU machines (tier C+). Speakers switch between English and Hindi, so live text detects the language per utterance. Live text is a draft; post-processing still produces the transcript. Read `docs/PLAN.md` sections 3, 4, 5 and 6.

## Goal
Live engine sessions started only from the Start Session dialog (or the Live captions switch, under the same rules), live text on the Record screen, the projector caption window and optional English translation on GPU machines, without slowing the recorder or breaking post-processing.

## Steps
1. Live feeder in the app: ring buffer, downmix to mono, resample to 16 kHz with rubato, and write f32 PCM to the live engine's stdin over a bounded queue (if full, drop the oldest chunk and count it).
2. `wb-engine live`:
   - Streaming mode (tier G): 1-second step; VAD gating; language detected per utterance among the session's languages (following the lock log), with the PLAN section 5 rules; greedy partial decode of the current utterance (at most 25 seconds); commit words that two consecutive hypotheses agree on (local agreement); final decode with beam 5 after 600 ms of silence or at 25 seconds; guards; times from the audio clock.
   - Utterance mode (tier C+): detect the language of each VAD utterance (up to 15 seconds) among the session's languages, then decode it once, greedy, with that language's near-live model and `audio_ctx` sized to the utterance plus a margin; no partials (show a listening indicator); guards.
3. Store: live segments go to revision `live-1` (draft), with `lang`, `lang_prob` and `mixed`. The rolling pass still produces `post-1`, which stays the default for Review and exports.
4. Starting and stopping: live starts only from the dialog's Live + post-process choice or the Record screen's Live captions switch, and only where `capability.json` clears every expected language of the session; otherwise both controls stay disabled with the reason. Switching off never interrupts recording. Language locks (Alt+1 to Alt+3) and Alt+4 apply at the next utterance and are logged.
5. Record screen live view per `design/mockups/record-live.html`; the Transcription card shows mode, live model, caption delay and memory.
6. Caption window: a second, borderless, full-screen Tauri window on the chosen display (cursor hidden, choice remembered): the last final line in `--ink-400`, the current line in white (`--fs-live-caption`), the partial at the end of the current line (streaming only), each segment in its own script, and the disclaimer "Automatic captions, generated offline. They may contain errors." in the footer bar with the logo lockup. Operator controls: Show or Hide, Freeze, Clear, text size, one or two lines. Flagged segments never appear.
7. Translation (D9, tier G only): when it is on and the segment language is hi or ur, run large-v3 with the translate task on each final segment and show the English line in `--ink-300` at `--fs-live-caption-sub`. Never use large-v3-turbo for translation; if large-v3 isn't loaded, say so on the Record screen, never on the projector.
8. Scheduler: on CPU-only machines, the batch engines pause (or drop to the profile's reduced threads) during near-live; on tier G the CPU engine keeps running while the live engine uses the GPU, unless the profile says otherwise.

## Verification
- On all three machines, a new session starts in Record + post-process with live off unless the operator changes it; live cannot be selected where the profile doesn't clear it (Playwright with fixture profiles, plus one manual run per machine).
- Tier G (Legion): replay a 30-minute recording that switches between English and Hindi (with an Urdu passage) through the virtual cable: p95 partial delay ≤ 1.5 s, p95 final delay ≤ 3 s, zero dropped chunks; live WER no more than 1.15 times the same model's batch WER on that audio; utterance languages correct at least 90% of the time.
- Tier C+ (the mid laptop if it qualifies): 30-minute English–Hindi replay in utterance mode: p95 caption delay ≤ 6 s; zero recorder overruns; the rolling pass still completes within its estimate.
- Caption window on a 1920x1080 display: readable from the back of a seminar hall (a person checks); caption delay no more than the Record screen's delay + 100 ms; screenshot beside `captions.png`.
- Force-kill the live engine: recording continues, live text resumes within 5 seconds, post-processing is unaffected.
- Screenshot of Record (live) beside `record-live.png`.
- `design-consistency-reviewer`, `asr-quality-auditor` and `offline-auditor` report PASS.

## If Blocked
- GPU delays are too high: use large-v3-turbo for English partials only, or lower `audio_ctx` for partials. Never lower post-processing quality.
- CPU near-live misses 6 seconds: cap utterances at 10 seconds, or stop offering near-live for that language on that machine and record why in the profile.
- Per-utterance detection flickers on short utterances: apply the 0.8 neighbour rule from PLAN section 5 before committing.
- Monitors can't be listed reliably: allow manual window placement and remember position and size.
