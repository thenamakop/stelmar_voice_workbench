"""Splits the errors of a day-simulation session by cause, from the transcript the run already wrote (Phase 5, item 3).
No decoding, no models: it reads the run folder's post-1 or post-2 JSONL, `day-audio.json` (the references) and the
fidelity analysis that places each utterance in the recording, the way `scripts/gates/day_report.py` does.

    eval/.venv/Scripts/python eval/panel_causes.py D:\\STELMAR-Gate\\day-sim\\day-emu-low-20261121-052323 ^
        D:\\STELMAR-Gate\\day-sim\\day-audio\\day-audio.json --out docs/evidence/phase-5/panel-causes.txt

Prints numbers only (no transcript text). Written to `docs/evidence/phase-5/`, the file is evidence: fix this script
and run it again instead of editing the file.

The causes, per block of the session (one block per source dataset):
  script mismatch   the reference is in a script the session does not write (Hindi speech in a Nastaliq session has a
                    Devanagari reference): no hypothesis word can ever match, whatever the engine heard.
  detection         the segments of the utterance mostly carry another language tag than the session's.
  normalisation     substitutions that disappear when more letter variants are folded (Urdu: ے ی, ھ, ؤ ئ أ إ آ).
  spelling variant  substitutions whose consonant skeleton is unchanged (a vowel, hamza or ghunna differs).
  recognition       what is left.
The consonant skeleton is a rough, script-independent comparison (Devanagari and Perso-Arabic letters mapped to one
class per consonant sound; vowels, semivowels and h dropped on both sides). It is INDICATIVE: it forgives vowel
differences that matter in Urdu, and it is calibrated here on the same session's Urdu block, which has a reference in
its own script.
"""

from __future__ import annotations

import argparse
import glob
import json
import sys
import unicodedata
from collections import Counter, defaultdict
from pathlib import Path

import jiwer

sys.path.insert(0, str(Path(__file__).resolve().parent))

import metrics  # noqa: E402
import normalize as N  # noqa: E402

# One class letter per consonant sound, Devanagari and Perso-Arabic alike, written as code points so that nothing
# depends on how an editor shows a combining mark. p and f are one class (Hindi फ is often f), z and j are one class.
_CLASSES = {
    "k": [0x0915, 0x0916, 0x06A9, 0x0643, 0x0642],  # क ख ک ك ق
    "g": [0x0917, 0x0918, 0x06AF, 0x063A],  # ग घ گ غ
    "c": [0x091A, 0x091B, 0x0686],  # च छ چ
    "j": [0x091C, 0x091D, 0x062C, 0x0698, 0x0632, 0x0630, 0x0636, 0x0638],  # ज झ ج ژ ز ذ ض ظ
    "s": [0x0938, 0x0936, 0x0937, 0x0633, 0x062B, 0x0635, 0x0634],  # स श ष س ث ص ش
    "t": [0x0924, 0x0925, 0x091F, 0x0920, 0x062A, 0x0679, 0x0637],  # त थ ट ठ ت ٹ ط
    "d": [0x0926, 0x0927, 0x0921, 0x0922, 0x062F, 0x0688],  # द ध ड ढ د ڈ
    "r": [0x0930, 0x0631, 0x0691],  # र ر ڑ
    "p": [0x092A, 0x092B, 0x067E, 0x0641],  # प फ پ ف
    "b": [0x092C, 0x092D, 0x0628],  # ब भ ب
    "m": [0x092E, 0x0645],  # म م
    "n": [0x0928, 0x0923, 0x0902, 0x0901, 0x0646, 0x06BA],  # न ण anusvara chandrabindu ن ں
    "l": [0x0932, 0x0933, 0x0644],  # ल ळ ل
}
_SOUND = {chr(cp): cls for cls, cps in _CLASSES.items() for cp in cps}
_NUKTA = "़"
_FLAP_SOURCES = ("ड", "ढ")  # ड ढ with a nukta are the flap ड़ ढ़, the sound of ر ڑ

# Letter variants an Urdu comparison may fold further than eval/normalize.py does today.
_FOLDS = {
    "ے ۓ -> ی": {"ے": "ی", "ۓ": "ی"},
    "ھ removed": {"ھ": None},
    "ؤ -> و, ئ -> ی, أ إ آ -> ا": {"ؤ": "و", "ئ": "ی", "أ": "ا", "إ": "ا", "آ": "ا"},
}


def skeleton(word: str) -> str:
    """The consonant skeleton of a word, one letter per sound ('' when it has none)."""
    out: list[str] = []
    prev = ""
    for ch in unicodedata.normalize("NFD", word):
        if ch == _NUKTA and prev in _FLAP_SOURCES and out:
            out[-1] = "r"
        else:
            out.append(_SOUND.get(ch, ""))
        prev = ch
    return "".join(out)


def skeleton_text(text: str) -> str:
    return " ".join(skeleton(w) or "_" for w in text.split())


def _errors(ref: str, hyp: str) -> tuple[int, int, int]:
    """Substitutions, deletions and insertions of a hypothesis against a reference (both normalised)."""
    n = len(ref.split())
    if n == 0:
        return 0, 0, len(hyp.split())
    if not hyp.split():
        return 0, n, 0
    out = jiwer.process_words(ref, hyp)
    return out.substitutions, out.deletions, out.insertions


def fold(text: str, mapping: dict[str, str | None]) -> str:
    return text.translate(str.maketrans({k: v for k, v in mapping.items()}))


def utterance_windows(session_dir: Path, manifest: Path, title: str, fid: Path) -> list[tuple[dict, list[dict]]]:
    """Each reference utterance with the segments whose centre falls in its window of the recording."""
    revisions = [session_dir / "post-2.jsonl", session_dir / "post-1.jsonl"]
    path = next((p for p in revisions if p.exists()), None)
    if path is None:
        raise SystemExit(f"{session_dir}: no post-1.jsonl")
    segs = sorted((json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()),
                  key=lambda s: s["start_ms"])
    segs = [s for s in segs if "text" in s]
    data = json.loads(manifest.read_text(encoding="utf-8-sig"))
    session = next(s for s in data["sessions"] if s["title"] == title)
    analysis = json.loads((fid / "reference-analysis.json").read_text(encoding="utf-8"))
    compare = json.loads((fid / "compare.json").read_text(encoding="utf-8"))
    shift = float(analysis["content_start_ms"]) + float(compare["segments"][0]["offset_frames"]) * 1000.0 / 48000.0
    rows = []
    for u in session["utterances"]:
        lo, hi = u["start_s"] * 1000.0 + shift, u["end_s"] * 1000.0 + shift
        rows.append((u, [s for s in segs if lo <= (s["start_ms"] + s["end_ms"]) / 2.0 <= hi]))
    return rows


def report(rows: list[tuple[dict, list[dict]]], session_lang: dict[str, str], P) -> None:
    """Prints the cause split of one session. `session_lang` maps a reference language to the tag the session expects."""
    blocks: dict[str, list[tuple[str, str, dict, list[dict]]]] = defaultdict(list)
    for u, win in rows:
        ref = N.normalize(u["text"])
        hyp = N.normalize(" ".join(s["text"] for s in win))
        blocks[u["dataset"]].append((ref, hyp, u, win))
    total = Counter()
    P("BLOCKS  utterance windows, current normaliser (eval/normalize.py); S D I are word errors")
    stats = {}
    for ds, items in sorted(blocks.items()):
        s = d = i = n = 0
        for ref, hyp, _u, _w in items:
            es, ed, ei = _errors(ref, hyp)
            s, d, i, n = s + es, d + ed, i + ei, n + len(ref.split())
        stats[ds] = (s, d, i, n)
        total.update({"errors": s + d + i, "words": n})
        P(f"  {ds}: {len(items)} utterances, reference words {n}, S{s} D{d} I{i} = {s + d + i} errors, WER {100 * (s + d + i) / n:.1f}%")
    P(f"  all blocks: {total['errors']} errors over {total['words']} reference words = {100 * total['errors'] / total['words']:.1f}% "
      "(utterance windows count insertions between utterances too, so this differs from the whole-text WER)")
    P("")
    P("SCRIPT  what the hypothesis is written in, against the script of the reference")
    for ds, items in sorted(blocks.items()):
        ref_scripts, hyp_scripts = Counter(), Counter()
        for ref, hyp, _u, _w in items:
            ref_scripts.update(N.word_script(w) for w in ref.split())
            hyp_scripts.update(N.word_script(w) for w in hyp.split())
        ref_main = ref_scripts.most_common(1)[0][0] if ref_scripts else None
        hyp_total = sum(hyp_scripts.values()) or 1
        in_ref = hyp_scripts.get(ref_main, 0)
        P(f"  {ds}: reference script {ref_main}; hypothesis words in the reference's script {in_ref} of {hyp_total} "
          f"({100 * in_ref / hyp_total:.0f}%); by script {dict(hyp_scripts)}"
          + ("  -> SCRIPT MISMATCH: no word can match" if in_ref == 0 and hyp_total > 10 else ""))
    P("")
    P("DETECTION  utterances whose segments are mostly tagged with another language than the session's")
    for ds, items in sorted(blocks.items()):
        bad = 0
        for _ref, _hyp, u, win in items:
            want = session_lang.get(u["lang"], u["lang"])
            dur = sum(s["end_ms"] - s["start_ms"] for s in win)
            if win and 2 * sum(s["end_ms"] - s["start_ms"] for s in win if s["lang"] != want) > dur:
                bad += 1
        P(f"  {ds} (session tag {session_lang.get(items[0][2]['lang'], items[0][2]['lang'])}): {bad} of {len(items)} utterances")
    P("")
    P("INDICATIVE SCRIPT-INDEPENDENT SCORE  consonant skeleton on both sides")
    for ds, items in sorted(blocks.items()):
        e = n = 0
        for ref, hyp, _u, _w in items:
            if not ref.split() or _u["lang"] == "en":
                continue  # the skeleton is for Hindi and Urdu references
            es, ed, ei = _errors(skeleton_text(ref), skeleton_text(hyp))
            e += es + ed + ei
            n += len(ref.split())
        if n:
            wer_now = 100 * sum(stats[ds][:3]) / stats[ds][3]
            P(f"  {ds}: skeleton WER {100 * e / n:.1f}% over {n} reference words (current-normaliser WER {wer_now:.1f}%)")
    P("  (a block whose reference is in the script the session writes shows how much the skeleton forgives: its two numbers are the calibration)")
    P("")
    for ds, items in sorted(blocks.items()):
        if not items or N.EXPECTED_SCRIPT.get(items[0][2]["lang"]) != N.ARABIC:
            continue
        base = [(r, h) for r, h, _u, _w in items if r.split() and N.word_script(r.split()[0]) == N.ARABIC]
        if not base:
            continue
        P(f"NORMALISATION AND SPELLING  {ds} (reference and output both Perso-Arabic)")
        def wer_of(f) -> float:
            e = n = 0
            for r, h in base:
                es, ed, ei = _errors(f(r), f(h))
                e += es + ed + ei
                n += len(r.split())
            return 100 * e / n
        P(f"  current normaliser: WER {wer_of(lambda t: t):.2f}%")
        for name, mapping in _FOLDS.items():
            P(f"  + {name}: WER {wer_of(lambda t, m=mapping: fold(t, m)):.2f}%")
        def all_folds(t: str) -> str:
            for m in _FOLDS.values():
                t = fold(t, m)
            return t
        P(f"  all folds together: WER {wer_of(all_folds):.2f}%")
        same = other = 0
        for r, h in base:
            out = jiwer.process_words(r, h)
            rw, hw = r.split(), h.split()
            for ch in out.alignments[0]:
                if ch.type == "substitute":
                    for a, b in zip(rw[ch.ref_start_idx:ch.ref_end_idx], hw[ch.hyp_start_idx:ch.hyp_end_idx]):
                        if skeleton(a) and skeleton(a) == skeleton(b):
                            same += 1
                        else:
                            other += 1
        s, d, i, _n = stats[ds]
        P(f"  substitutions {s}: spelling variants (same consonant skeleton) {same}, other {other}; deletions {d}; insertions {i}")
        P("")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run", type=Path, help="the day-simulation run folder")
    ap.add_argument("manifest", type=Path, help="day-audio.json")
    ap.add_argument("--title", default="Panel")
    ap.add_argument("--fid", default="fid-4", help="the session's fidelity folder in the run")
    ap.add_argument("--hi-as", default="ur", help="the tag the session expects for Hindi speech (Nastaliq session: ur)")
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    dirs = glob.glob(str(args.run / "rec" / "*" / "*"))
    session_dir = next(Path(d) for d in dirs if Path(d).name.endswith(args.title.lower().replace(" ", "-")))
    rows = utterance_windows(session_dir, args.manifest, args.title, args.run / args.fid)
    lines: list[str] = []
    P = lines.append
    P(f"Error causes of '{args.title}' in {args.run.name} (Phase 5, item 3). Numbers only; written by eval/panel_causes.py.")
    P(f"revision file: {'post-2' if (session_dir / 'post-2.jsonl').exists() else 'post-1'}; utterances {len(rows)}; "
      f"session tags: Hindi speech expected as '{args.hi_as}'")
    P("")
    report(rows, {"hi": args.hi_as, "ur": "ur", "en": "en"}, P)
    text = "\n".join(lines) + "\n"
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text, encoding="utf-8")
    sys.stdout.buffer.write(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
