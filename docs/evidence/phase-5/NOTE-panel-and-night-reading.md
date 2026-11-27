# Reading of `panel-causes.txt` and `night-speed.txt` (Phase 5, items 3 and 7)

Written by the coding agent (Claude Code, Window 1) on 23 Nov 2026. The two `.txt` files beside this note are written by
scripts (`eval/panel_causes.py`, `scripts/gates/night_speed.py`) and are never edited by hand; this note is a person's or
an agent's reading of them and can be wrong. Numbers come from the 21 Nov 2026 day simulation
(`day-emu-low-20261121-052323`, emulation profile `emu-low`, Legion GPU) and the 23 Nov smoke run.

## Item 3: why the Panel scored 67.3%

1. **The Panel is not English and Urdu.** `scripts/gates/make_day_audio_full.py` builds it from English (Svarah), Hindi
   (FLEURS Hindi, 224 utterances, 59% of the reference words) and 10 minutes of Urdu (Kathbath), and starts it as a
   Nastaliq session: Hindi and Urdu are written in Urdu script (the organiser's choice, GATES row of 13 Nov 2026).
   The plan's wording ("English and Urdu") was wrong.
2. **Script mismatch is the main cause, and it is a scoring artefact.** The Hindi block's references are Devanagari; the
   hypothesis is 93% Urdu script and 7% Latin, so no hypothesis word can equal a reference word (0 of 5,943). Its 108.0%
   WER (6,123 errors) is 84% of all word errors of the three blocks. It measures nothing about recognition.
3. **A script-independent look (consonant skeleton, indicative only).** The Hindi block comes to about 33% and the
   Urdu block, which has a reference in its own script, to 32.9% (its WER is 43.3%: the skeleton forgives about ten
   points of vowel and spelling differences). So the Hindi speech was recognised at roughly the level of the Urdu speech;
   the true number needs an Urdu-script reference for the Hindi clips, or a Hindi session scored in Devanagari (the Kathbath
   Hindi and Urdu rows of `docs/GATES.md` are the standing measurements).
4. **Detection** is small: 19 of 224 Hindi utterances, 2 of 81 Urdu and 2 of 227 English utterances were mostly tagged with
   another language than the session's. The Hindi block has 412 Latin words (7% of its hypothesis words); 34 of its 325 segments carry the tag en, which probably accounts for many of them (not checked word by word).
5. **Normalisation** is small: folding more Urdu letter variants (ے, ھ, ؤ ئ أ إ آ) moves the Urdu block from 43.3% to
   41.8%, at most 1.5 points. 82 of its 194 substitutions keep the consonant skeleton (a vowel, hamza or ghunna differs):
   real differences, not spelling the scorer could fold away.
6. **English** (Svarah, 25.4%) is mostly deletions (505 of 785 errors, 16% of its reference words), not substitutions:
   speech that produced no text. Digits cause 23 of its 117 substitutions. This is worth its own look at the chunk flags
   (`low_coverage`, `retried` and dropped chunks) before any model change.

Recommendation: do not change a model for the Panel's 67.3%. Change how the day report scores a Nastaliq session (score
the Hindi block with the skeleton, or give the Hindi clips an Urdu-script reference), and look at the English block's
deletions. Both are outside the frozen paths.

## Item 7: night plan against the night units

In the full day the night plan said 5.6 h (ready by 23:40 IST) at 2.44 times the profile's night speed, a factor learned
from five rolling units. The 19 night units that ran needed 1.14 engine seconds per audio second, the plan assumed 1.86:
actual over planned 0.62. Rolling units took 1.16 on the same GPU, so the rolling-learned factor includes the recorder
running beside them, which the night does not have. The plan was too slow by about 1.6 times; on a 13.5 h night it
accepts up to 436 min of night audio where the units' speed would have fitted 708 min. That error is on the safe side, and
the owner decided on 23 Nov 2026 to keep it conservative until the laptops give data. The smoke5 run (night factor 1.74)
ended its night pass by the 8-minute time box before any night unit finished, so it says nothing about night speed.

Proposal: change nothing now. When the laptops run the gate pack's day simulation, take the ratio actual over planned for
the night units (0.62 here) per laptop; fit the night factor from it only if it is stable across both laptops.
