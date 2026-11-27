# Phase 6: Sessions, review, export and archive

## Context
Transcripts now arrive through post-processing (revisions `post-1` and `post-2`), with live drafts to come in Phase 7. This phase gives organisers the tools to plan sessions, correct transcripts against the audio and share them. Read `docs/PLAN.md` sections 8 and 9.

## Goal
A Sessions screen seeded from the STELMAR programme, a Review editor synced with the audio across parts, revisions, exports in TXT, SRT, WebVTT, JSON and DOCX, session and day archives with checksums, and a backup check.

## Steps
1. Sessions screen: load `docs/programme.json` (8 and 9 February 2027, 13 items, 8 marked for transcription); group by day; one row per session from `SessionItem` with status Badges (Not Started, Recording, Transcribing, Transcribed, Reviewed) and the mode and languages chosen in the Start Session dialog; "New Session" for anything else; parallel-track items become one session per hall.
2. Review editor: segment list with inline editing; audio player that seeks across parts using `parts.json`; shortcuts (Space play or pause, Alt+Left and Alt+Right jump 5 seconds, Ctrl+Enter next segment); speaker labels; bookmarks from the recording log; search and replace; revision history (live draft, `post-1`, `post-2`, edits) with restore. Every save is a new revision; nothing is overwritten.
3. Hindi, Urdu and mixed Hindi–English editing follows the Phase 5 rendering rules; a segment's language can be corrected, and the correction is kept as an edit.
4. Exporters in Rust from the chosen revision: TXT (session header, speaker and time per paragraph), SRT and WebVTT (at most 2 lines of about 42 characters for Latin script, wrapped by words; Unicode text as-is for Hindi and Urdu), JSON (all segment fields), DOCX (docx-rs; STELMAR title block; Urdu paragraphs marked right to left).
5. Export dialog (DS `Dialog` with a `Checkbox` per format and a destination picker); exports also land in the session's `exports/` folder.
6. Archive: zip a session or a whole day (audio parts, `parts.json`, JSONL revisions, `transcript.db`, exports, `session.json`, `manifest.sha256`), then verify every hash after writing. The Phase 4 import accepts an unpacked archive.
7. Backup check: compare primary and mirror trees by SHA-256 and show differences on the Settings screen.
8. Carried over from the Phase 5 design review (owner, 27 Nov 2026), done with the Sessions screen:
   - **Wire `initialLanguages` of `StartSessionDialog`.** The prop exists (Phase 5, groundwork) and nothing passes it: the Record screen has no programme entry, so an Urdu session would still open on English + Hindi in Devanagari. The Sessions screen opens the dialog from a programme row and passes that row's languages and script. Test: a row listed in Urdu opens the dialog with Urdu selected.
   - A second like-for-like 1536x864 screenshot of the Start Session dialog, opened from a Sessions row, beside its mockup.
   - The Speed row in the Transcription card wraps: keep it on one line at 1536x864.
   - Counts on the Transcript and Bookmarks tabs, as in the mockup.
   - Font sizes 17 and 19 px are not in `design/tokens/typography.css`: use the token sizes (the token files are frozen; ask before adding one).
   - Copy: write "needs 1.00×" as the mockup does, and say the "FASTER" sentence once.

## Verification
- End to end (Rust integration tests plus Playwright with `mockIPC`): record, rolling pass, night pass, edit, restart; edits and revisions are intact.
- SRT and VTT files parse with a parser test and play with the WAV in VLC (a person checks one file per language).
- DOCX opens in Word or LibreOffice with Devanagari, Urdu and mixed Hindi–English text intact and Urdu right to left.
- Archive verification passes; changing one byte makes it fail; an unpacked archive imports on another machine and opens in Review with the same segments and revisions.
- 1536x864 screenshots of Sessions and Review.
- `design-consistency-reviewer` and `offline-auditor` report PASS.

## If Blocked
- The DOCX library can't mark paragraphs right to left: set the paragraph `bidi` property directly in the XML; failing that, export Urdu in TXT, SRT and VTT only and note it.
- Seeking across part boundaries is unreliable: build the player's timeline from `parts.json` durations rather than file headers.
