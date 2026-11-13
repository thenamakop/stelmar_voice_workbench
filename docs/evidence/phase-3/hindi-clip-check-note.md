# Hindi clip check: reading of the numbers

Written by Claude Code (the coding agent), 13 Nov 2026, after `scripts/gates/hindi-clip-check.ps1` ran on the Legion
(emu-low, Vulkan UHD, large-v3-turbo q5_0, Hindi). The numbers are in `hindi-clip-check.txt`, which a script wrote.
This note is a reading of them, not evidence. The owner decides what changes.

## What the numbers say

| Clip | Timed RTF (median of 3) | Against the 0.75 emulated bar |
| --- | --- | --- |
| (a) bundled Hindi clip, first 30 s (what the quick check times) | 0.827 (0.825 to 0.855) | misses |
| (b) the same clip, 60 s (what the full check times) | 0.696 (0.687 to 0.697) | meets |
| (c) the Phase 2 speed-table Hindi clip, 60.2 s | 0.691 (0.688 to 0.693) | meets |

* **It is the excerpt length, not the clip.** The bundled clip and the Phase 2 clip hold the same audio over the
  bundled clip's whole 60 s (the Phase 2 clip is 0.22 s longer), so (b) against (c) differs by 0.8%. The same clip at 30 s
  against 60 s differs by 18.7%, far outside the run-to-run spread (3 runs each, ranges under 4%).
* **The Phase 2 figure of 0.734 is not the machine slowing down.** The 60 s clip now gives 0.691, 6% faster than the
  Phase 2 table. The quick check's 0.83 was a 30 s figure compared with a 60 s bar.
* **Why 30 s is slower (a plausible reading, not proved).** The clip is denser at the start. Characters of reference text
  per second of clip: the first 30 s about 10.1, the whole clip 8.77 (80th percentile among the 413 sixty-second windows
  of the FLEURS Hindi test set, median 8.17), so the second half is sparse (about 7.5). A 15% density difference is the
  same size as the 19% speed difference. Per second of speech the bundled clip is typical (12.67 against a median of
  12.62), so the clip is not unusually fast speech; it is unusually little silence. A 30 s cut also has fewer VAD chunks
  to average over (3 segments against 5) and pays the same per-chunk costs.
* The same start-of-clip effect exists for Urdu (first 30 s about 11.8 characters per second, the whole clip 10.5, 88th
  percentile) and not for English (10.0 against 10.1). The quick check's Urdu figure (0.676 on 30 s) against the full check's 0.653 on 60 s (+3.6%, under the 10% flag) shows
  the Urdu effect is small on this clip; the Hindi, English and Hindi-medium cells differ by 20 to 35% (see the GATES row of 14 Nov 2026).

## What I propose (nothing is changed until the owner answers)

1. **Time Hindi (and Urdu) on a window that represents the clip, not on its first 30 s.** Two ways, about the same cost:
   * use the whole 60 s for the rolling cells of the language: about 25 s more on the UHD at emu-low for the Hindi turbo
     cell, and it is then exactly the full check's cell, so quick and full agree by construction; or
   * average two 30 s excerpts from different parts of the clip (0 to 30 s and 30 to 60 s): same audio, same cost, and
     it keeps one code path for every language.
2. **Do not pick a different clip.** The bundled clip is typical (80th percentile by characters per second, median by
   speech density); the problem is the excerpt, and a "more typical" clip would only move the same error to another place.
3. **Limits of this check:** one clip, one model, one machine. It shows the 30 s figure is pessimistic for this clip. It does
   not show that every 30 s excerpt is. The full check's quick-versus-full comparison will show the same difference for the
   other cells (English, Urdu, medium).
4. **Cost against the quick-check target.** A 60 s excerpt for the three languages adds about 25 s per cell on emu-low
   (about 75 s for three turbo cells), which would push the emu-low quick check back toward the 360 s target's edge.
   Hindi alone is the cell that decides a verdict (it is the one at the bar), so I would apply it to a language only when its
   30 s figure lands within 25% of the bar, and keep 30 s elsewhere. That is a rule about the measured number, not about a
   class of machine.
