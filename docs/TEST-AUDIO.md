# Local test audio

**When:** later, once most of the development work is done (after Phase 4 is merged), and before the local-set parts of Phase 5. Until then evaluation runs on the public sets only (Phase 5a). This task is tracked here so it is not forgotten.

## Why it is needed

Picking models and tuning language detection needs an answer key: recordings with transcripts a person has checked, so the app's output can be compared word by word (word error rate). Public datasets are not enough, because STELMAR audio differs from them.

| Public datasets have | STELMAR talks will have |
| --- | --- |
| Clean read speech, one language per clip | Speakers switching between English and Hindi, even mid-sentence |
| Studio-like microphones | A hall microphone, the mixer and room echo |
| General vocabulary | Academic terms, "STELMAR", "BML Munjal", speaker names |

The local sets decide:

1. Which model wins for English, Hindi and mixed speech in the real conditions (Phase 5).
2. Whether per-sentence language detection works, and its thresholds (0.8, 0.2 and 0.5 in `docs/PLAN.md` section 5).
3. Whether English words inside Hindi sentences come out readable (decision D14).
4. That the chosen setup beats plain Whisper on this audio (a Phase 5 gate).

## What to record (about 75 minutes)

| Set | Length | Notes |
| --- | --- | --- |
| English | 15 minutes | Indian-accented academic talk, 2 or 3 speakers |
| Hindi | 15 minutes | Natural spoken Hindi, not read from a script |
| Urdu | 15 minutes | Skip if no speaker is available |
| English–Hindi mixed | 30 minutes | The most important set: speakers present a topic naturally and switch whenever it feels normal |

## Steps

1. **Record** in a seminar room through a microphone and mixer if possible (a phone or laptop microphone is the fallback); 48 kHz WAV; a one-line written consent from each speaker.
2. **Cut** into clips of 1 to 3 minutes so the work can be shared.
3. **Draft** each clip with Whisper large-v3 or the Hinglish model on the Legion. Correcting a draft takes about 3 to 4 times the audio length; typing from scratch takes 6 to 10 times.
4. **Correct by ear**: fix every word you hear, not only the ones that look wrong, or the answer key flatters the drafting model. Hindi in Devanagari; English words in Latin script even inside Hindi sentences; no added filler words; numbers as spoken.
5. **Second pass** by a different person.
6. **Save** a `.txt` per clip with the same name as its `.wav`, into `eval/local/` (git-ignored, private).

With 3 or 4 volunteers this is about 2 to 3 hours each. If time runs out, even 10 minutes of the mixed set is far better than none.
