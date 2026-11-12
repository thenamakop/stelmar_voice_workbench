"""Code-switching gate (Phase 2): does `wb-engine batch` pick the right language for each chunk?

    eval/.venv/Scripts/python scripts/gates/codeswitch.py prepare --scratch SCRATCH
    # you label the chunks in SCRATCH/labelling/sheet.html and save labels.json next to it
    eval/.venv/Scripts/python scripts/gates/codeswitch.py score   --scratch SCRATCH --out EVIDENCE.txt
    eval/.venv/Scripts/python scripts/gates/codeswitch.py control --scratch SCRATCH --out EVIDENCE.txt

How the 10 chunks are chosen (fixed here, before anyone labels and before any language detection):
  1. CoSHE-Eval utterances whose reference mixes scripts, of at least 25 s, and whose reference is 30% to
     70% Latin words (the reference is read only to choose clips; it is never shown to the labeller and
     never written to the evidence).
  2. Three clips, with a fixed seed.
  3. Their chunks come from `wb-engine chunks` (the VAD alone: no model, no detection), 2 s or longer.
  4. The chunks are put in a fixed low-discrepancy order (bit-reversal over the list sorted by clip and
     time), and the first 10 are the sample. That spreads the sample over the clips and over time; any
     replacement (below) is the next one in the same order.
  5. After labelling, if fewer than 3 chunks are led by English or fewer than 3 by Hindi, the next
     chunk in the order replaces a chunk of the over-represented language (the latest in the order),
     until both have 3. `swap` does this and records it; detection has not run at that point.

Labels per chunk: en, hi or mixed; for mixed also the language most of the chunk is in. A mixed chunk
is scored against that main language. The committed evidence holds only clip ids, chunk times, the
labels, the detected language and probability and the counts: no audio and no transcript text
(CoSHE-Eval is CC BY-NC).

The control clip is 10 FLEURS test utterances, alternating 5 English and 5 Hindi, joined with 0.8 s of
silence; its labels are known by construction.
"""

from __future__ import annotations

import argparse
import html
import json
import random
import re
import subprocess
import sys
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
ENGINE = REPO / "src-tauri" / "binaries" / "wb-engine-avx2-x86_64-pc-windows-msvc.exe"
SEED = 20261110
MIN_CHUNK_S = 2.0
SAMPLE = 10


# ----------------------------------------------------------------------------------- helpers

def latin_share(text: str) -> float:
    words = text.split()
    if not words:
        return 0.0
    return sum(1 for w in words if re.search(r"[A-Za-z]", w)) / len(words)


def script_counts(text: str) -> dict[str, int]:
    c = {"latin": 0, "devanagari": 0, "arabic": 0}
    for ch in text:
        if "ऀ" <= ch <= "ॿ":
            c["devanagari"] += 1
        elif "؀" <= ch <= "ۿ":
            c["arabic"] += 1
        elif ch.isascii() and ch.isalpha():
            c["latin"] += 1
    return c


def read_wav(path: Path) -> tuple[bytes, int]:
    with wave.open(str(path), "rb") as w:
        assert w.getnchannels() == 1 and w.getsampwidth() == 2, f"{path}: expected 16-bit mono"
        return w.readframes(w.getnframes()), w.getframerate()


def write_wav(path: Path, frames: bytes, rate: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(frames)


CONTROL_RMS_DBFS = -26.0


def to_level(frames: bytes, dbfs: float = CONTROL_RMS_DBFS) -> bytes:
    """Scales 16-bit samples to a common RMS level. Most FLEURS English clips are recorded 55 to 70 dB
    below full scale (median RMS 33 of 32768), which the VAD skips when loud speech comes before them;
    a session is recorded at a sensible level, so the control levels every utterance the same way."""
    import array
    import math

    s = array.array("h")
    s.frombytes(frames)
    if not s:
        return frames
    rms = math.sqrt(sum(x * x for x in s) / len(s))
    if rms < 1:
        return frames
    g = min(10 ** (dbfs / 20) * 32768 / rms, 32767 / max(1, max(abs(x) for x in s)))
    return array.array("h", (int(round(x * g)) for x in s)).tobytes()


def bit_reversal_order(n: int) -> list[int]:
    """0..n-1 in van der Corput order: spread over the whole range, any prefix evenly covers it."""
    if n == 0:
        return []
    bits = max(1, (n - 1).bit_length())
    order = sorted(range(1 << bits), key=lambda i: int(format(i, f"0{bits}b")[::-1], 2))
    return [i for i in order if i < n]


def engine_chunks(wav: Path, out: Path) -> list[dict]:
    out.parent.mkdir(parents=True, exist_ok=True)
    r = subprocess.run([str(ENGINE), "chunks", "--file", str(wav), "--out", str(out)], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"wb-engine chunks failed ({r.returncode}): {r.stderr[-400:]}")
    return json.loads(out.read_text(encoding="utf-8"))["chunks"]


def engine_batch(wav: Path, out: Path, langs: str, models: str, reuse: bool = False) -> list[dict]:
    """Runs `wb-engine batch`; with `reuse`, reads the segments an earlier run left in `out` (no decoding)."""
    out.parent.mkdir(parents=True, exist_ok=True)
    if reuse:
        if not out.exists():
            raise SystemExit(f"--reuse: {out} does not exist")
        return [json.loads(line) for line in out.read_text(encoding="utf-8").splitlines() if line.strip()]
    r = subprocess.run(
        [str(ENGINE), "batch", "--file", str(wav), "--models", models, "--langs", langs, "--out", str(out),
         "--settings", json.dumps({"sentence_split": True})],  # the chunks of a multi-language session; the same as `chunks`
        capture_output=True, text=True, encoding="utf-8", errors="replace",
    )
    if r.returncode != 0:
        raise SystemExit(f"wb-engine batch failed ({r.returncode}): {r.stderr[-400:]}")
    return [json.loads(line) for line in out.read_text(encoding="utf-8").splitlines() if line.strip()]


# ----------------------------------------------------------------------------------- prepare

def cmd_prepare(a: argparse.Namespace) -> None:
    scratch = Path(a.scratch)
    set_dir = REPO / "eval" / "data" / "coshe-eval"
    utts = [json.loads(line) for line in (set_dir / "manifest.jsonl").read_text(encoding="utf-8").splitlines() if line.strip()]
    usable = [u for u in utts if u["mixed"] and u["duration_s"] >= 25 and 0.30 <= latin_share(u["text"]) <= 0.70]
    rule = "mixed reference, >= 25 s, 30% to 70% Latin words"
    if len(usable) < 3:
        usable = [u for u in utts if u["duration_s"] >= 25]
        rule = "any utterance >= 25 s (the reference does not separate the scripts)"
    rng = random.Random(SEED)
    picked = sorted(rng.sample(sorted(usable, key=lambda u: u["id"]), 3), key=lambda u: u["id"])
    candidates: list[dict] = []
    for u in picked:
        wav = set_dir / u["audio"]
        for c in engine_chunks(wav, scratch / "chunks" / f"{u['id']}.json"):
            if (c["end_ms"] - c["start_ms"]) / 1000 >= MIN_CHUNK_S:
                candidates.append({"clip": u["id"], "start_ms": c["start_ms"], "end_ms": c["end_ms"], "chunk": c["chunk"]})
    candidates.sort(key=lambda c: (c["clip"], c["start_ms"]))
    order = bit_reversal_order(len(candidates))
    ranked = [candidates[i] for i in order]
    selection = {"seed": SEED, "rule": rule, "clips": [u["id"] for u in picked], "ranked": ranked, "sample": SAMPLE, "swaps": []}
    (scratch / "selection.json").write_text(json.dumps(selection, indent=2), encoding="utf-8")
    write_sheet(scratch, selection, set_dir, ranked[:SAMPLE])
    print(f"{len(candidates)} chunks of 3 clips; sheet with the first {SAMPLE}: {scratch / 'labelling' / 'sheet.html'}")


def write_sheet(scratch: Path, selection: dict, set_dir: Path, chosen: list[dict]) -> None:
    snippets = scratch / "labelling" / "snippets"
    snippets.mkdir(parents=True, exist_ok=True)
    rows = []
    for n, c in enumerate(chosen):
        utt = next(u for u in json.loads("[" + ",".join((set_dir / "manifest.jsonl").read_text(encoding="utf-8").splitlines()) + "]") if u["id"] == c["clip"])
        frames, rate = read_wav(set_dir / utt["audio"])
        a, b = c["start_ms"] * rate // 1000 * 2, c["end_ms"] * rate // 1000 * 2
        write_wav(snippets / f"{n:02d}.wav", frames[a:b], rate)
        rows.append(
            f'<section><h3>Chunk {n + 1}</h3><audio controls preload="none" src="snippets/{n:02d}.wav"></audio>'
            f'<div class="r"><label><input type="radio" name="c{n}" value="en"> English</label> '
            f'<label><input type="radio" name="c{n}" value="hi"> Hindi</label> '
            f'<label><input type="radio" name="c{n}" value="mixed"> Mixed, mostly '
            f'<select name="m{n}"><option value="">(choose)</option><option value="en">English</option><option value="hi">Hindi</option></select></label></div></section>'
        )
    page = f"""<!doctype html><meta charset="utf-8"><title>Label the chunks</title>
<style>body{{font:16px system-ui;max-width:46rem;margin:2rem auto;padding:0 1rem}}section{{border-top:1px solid #ccc;padding:.8rem 0}}
.r label{{margin-right:1.2rem}}button{{font-size:1rem;padding:.5rem 1rem}}</style>
<h1>Label {len(chosen)} chunks</h1>
<p>Listen to each chunk. English: only English is spoken. Hindi: only Hindi. Mixed: both languages are in the chunk;
say which one most of it is in. Then press the button; save the file as <b>labels.json</b> in the folder this page is in.</p>
{''.join(rows)}
<p><button onclick="save()">Save labels.json</button> <span id="msg"></span></p>
<script>
function save() {{
  const out = [];
  for (let i = 0; i < {len(chosen)}; i++) {{
    const r = document.querySelector('input[name=c' + i + ']:checked');
    const m = document.querySelector('select[name=m' + i + ']').value;
    if (!r) {{ document.getElementById('msg').textContent = 'Chunk ' + (i + 1) + ' has no answer yet.'; return; }}
    if (r.value === 'mixed' && !m) {{ document.getElementById('msg').textContent = 'Chunk ' + (i + 1) + ': choose the main language.'; return; }}
    out.push({{ i: i, label: r.value, main: r.value === 'mixed' ? m : r.value }});
  }}
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([JSON.stringify({{ labels: out }}, null, 2)], {{ type: 'application/json' }}));
  a.download = 'labels.json'; a.click();
  document.getElementById('msg').textContent = 'Saved.';
}}
</script>"""
    (scratch / "labelling" / "sheet.html").write_text(page, encoding="utf-8")


# ------------------------------------------------------------------------------------ score

def load_labels(scratch: Path, labels_path: Path | None) -> list[dict]:
    p = labels_path or (scratch / "labelling" / "labels.json")
    return json.loads(p.read_text(encoding="utf-8"))["labels"]


def current_sample(selection: dict) -> list[dict]:
    chosen = selection["ranked"][: selection["sample"]]
    # Apply recorded swaps: out (by index in the sample) -> in (rank in the ranked list).
    for s in selection["swaps"]:
        chosen[s["out_index"]] = selection["ranked"][s["in_rank"]]
    return chosen


def cmd_swap(a: argparse.Namespace) -> None:
    scratch = Path(a.scratch)
    selection = json.loads((scratch / "selection.json").read_text(encoding="utf-8"))
    labels = load_labels(scratch, Path(a.labels) if a.labels else None)
    led = lambda lab: lab["main"]  # noqa: E731
    counts = {"en": sum(1 for lab in labels if led(lab) == "en"), "hi": sum(1 for lab in labels if led(lab) == "hi")}
    if counts["en"] >= 3 and counts["hi"] >= 3:
        print(f"both languages lead at least 3 chunks ({counts}); nothing to swap")
        return
    over = "en" if counts["en"] > counts["hi"] else "hi"
    next_rank = SAMPLE + len(selection["swaps"])
    out_index = max(i for i, lab in enumerate(labels) if led(lab) == over)
    selection["swaps"].append({"out_index": out_index, "in_rank": next_rank, "reason": f"counts {counts}, {over} over-represented"})
    (scratch / "selection.json").write_text(json.dumps(selection, indent=2), encoding="utf-8")
    set_dir = REPO / "eval" / "data" / "coshe-eval"
    write_sheet(scratch, selection, set_dir, current_sample(selection))
    print(f"replaced chunk {out_index + 1} with the next in the order; relabel only that chunk (new sheet written)")


def segment_for(events: list[dict], c: dict) -> dict | None:
    """The engine's answer for a sampled chunk. One segment that covers the chunk is its answer. A chunk the
    retry ladder split (or any chunk cut into several segments) is answered by the language that covers
    most of its time (segments with no text, flagged `unclear`, still carry the detected language); the
    segments must cover at least half of the chunk, else there is no answer."""
    segs = [e for e in events if e.get("event") == "segment"]
    for e in segs:
        if e["start_ms"] <= c["start_ms"] + 400 and e["end_ms"] >= c["end_ms"] - 400:
            return e

    def overlap(e: dict) -> int:
        return max(0, min(e["end_ms"], c["end_ms"]) - max(e["start_ms"], c["start_ms"]))

    inside = [e for e in segs if overlap(e) > 0]
    if sum(overlap(e) for e in inside) < (c["end_ms"] - c["start_ms"]) / 2:
        return None
    cover: dict[str, int] = {}
    for e in inside:
        cover[e["lang"]] = cover.get(e["lang"], 0) + overlap(e)
    lang = max(cover, key=lambda k: cover[k])
    mine = [e for e in inside if e["lang"] == lang]
    best = max(mine, key=overlap)
    prob = sum(e["lang_prob"] * overlap(e) for e in mine) / max(1, sum(overlap(e) for e in mine))
    return {**best, "lang": lang, "lang_prob": prob, "split_into": len(inside)}


def cmd_score(a: argparse.Namespace) -> None:
    scratch = Path(a.scratch)
    selection = json.loads((scratch / "selection.json").read_text(encoding="utf-8"))
    labels = load_labels(scratch, Path(a.labels) if a.labels else None)
    sample = current_sample(selection)
    set_dir = REPO / "eval" / "data" / "coshe-eval"
    models = a.models
    events_by_clip: dict[str, list[dict]] = {}
    for clip in selection["clips"]:
        utt = next(json.loads(line) for line in (set_dir / "manifest.jsonl").read_text(encoding="utf-8").splitlines() if json.loads(line)["id"] == clip)
        events_by_clip[clip] = engine_batch(set_dir / utt["audio"], scratch / f"batch-{clip}.jsonl", "en,hi", models, reuse=a.reuse)
    lines = [
        "Code-switching gate: per-chunk language against hand labels (Phase 2)",
        f"engine: {ENGINE.name}; models: {models}; languages en,hi; sentence_split on (the chunks the labeller heard come from the same chunker); clips {', '.join(selection['clips'])}; selection seed {selection['seed']}; rule: {selection['rule']}",
        "no audio and no transcript text in this file (CoSHE-Eval is CC BY-NC)",
        "",
        "n | clip | start-end ms | your label (main) | detected | prob | result",
    ]
    ok_pure = tot_pure = ok_mixed = tot_mixed = 0
    translated = missing = 0
    led = {"en": 0, "hi": 0}
    for i, (c, lab) in enumerate(zip(sample, labels)):
        e = segment_for(events_by_clip[c["clip"]], c)
        led[lab["main"]] += 1
        if e is None:
            missing += 1
            lines.append(f"{i + 1} | {c['clip']} | {c['start_ms']}-{c['end_ms']} | {lab['label']} ({lab['main']}) | no segment | | MISS (nothing decoded for this chunk)")
            if lab["label"] == "mixed":
                tot_mixed += 1
            else:
                tot_pure += 1
            continue
        good = e["lang"] == lab["main"]
        sc = script_counts(e["text"])
        # A Hindi-led chunk decoded as English words only is a translation.
        if lab["main"] == "hi" and e["lang"] == "en" and sc["devanagari"] == 0:
            translated += 1
        if lab["label"] == "mixed":
            tot_mixed += 1
            ok_mixed += good
        else:
            tot_pure += 1
            ok_pure += good
        lines.append(f"{i + 1} | {c['clip']} | {c['start_ms']}-{c['end_ms']} | {lab['label']} ({lab['main']}) | {e['lang']} | {e['lang_prob']:.2f} | {'OK' if good else 'WRONG'}"
                     + (f" (the chunk came back as {e['split_into']} segments; the language covering most of it)" if e.get("split_into") else ""))
    total = tot_pure + tot_mixed
    ok = ok_pure + ok_mixed
    lines += [
        "",
        f"result: {ok} of {total} chunks match ({ok_pure} of {tot_pure} pure, {ok_mixed} of {tot_mixed} mixed, scored against the main language); chunks with nothing decoded: {missing}",
        f"mixed chunks in the sample: {tot_mixed}; chunks led by English {led['en']}, by Hindi {led['hi']}; swaps made before detection: {len(selection['swaps'])}",
        f"Hindi-led chunks that came back as English words only (translated): {translated}",
        f"baseline for Phase 5 (the accuracy gate of 9 of 10 and no translated chunk moved to Phase 5 on 11 Nov 2026): {'at the bar' if ok >= 9 and translated == 0 and total == SAMPLE else 'below the bar'}",
    ]
    Path(a.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


# ----------------------------------------------------------------------------------- control

def cmd_control(a: argparse.Namespace) -> None:
    scratch = Path(a.scratch)
    rng = random.Random(SEED + 1)
    pools = {}
    for lang in ("en", "hi"):
        d = REPO / "eval" / "data" / f"fleurs-{lang}"
        us = [json.loads(line) for line in (d / "manifest.jsonl").read_text(encoding="utf-8").splitlines() if line.strip()]
        us = [u for u in us if 4 <= u["duration_s"] <= 14]
        pools[lang] = (d, rng.sample(sorted(us, key=lambda u: u["id"]), 5))
    order = []
    for i in range(5):
        order += [("en", pools["en"][1][i], pools["en"][0]), ("hi", pools["hi"][1][i], pools["hi"][0])]
    frames = b""
    rate = 16000
    spans = []
    silence = b"\x00\x00" * int(rate * 0.8)
    pos = 0
    for lang, u, d in order:
        f, r = read_wav(d / u["audio"])
        assert r == rate
        f = to_level(f)
        spans.append({"lang": lang, "id": u["id"], "start_ms": pos * 1000 // rate, "end_ms": (pos + len(f) // 2) * 1000 // rate})
        frames += f + silence
        pos += len(f) // 2 + len(silence) // 2
    wav = scratch / "control.wav"
    write_wav(wav, frames, rate)
    events = engine_batch(wav, scratch / "control-batch.jsonl", "en,hi", a.models, reuse=a.reuse)
    segs = [e for e in events if e.get("event") == "segment"]
    lines = [
        "Code-switching control: 10 FLEURS utterances alternating English and Hindi, joined with 0.8 s of silence (Phase 2)",
        f"engine: {ENGINE.name}; models: {a.models}; sentence_split on (as in a multi-language session); every utterance scaled to {CONTROL_RMS_DBFS:.0f} dBFS RMS first (peak limited), see to_level; labels known by construction; no transcript text in this file",
        "",
        "n | utterance | start-end ms | true | detected (the segment covering most of it) | prob | result",
    ]
    ok = 0
    for n, s in enumerate(spans):
        best = max(segs, key=lambda e: max(0, min(e["end_ms"], s["end_ms"]) - max(e["start_ms"], s["start_ms"])), default=None)
        overlap = 0 if best is None else max(0, min(best["end_ms"], s["end_ms"]) - max(best["start_ms"], s["start_ms"]))
        if best is None or overlap < (s["end_ms"] - s["start_ms"]) / 2:
            lines.append(f"{n + 1} | {s['id']} | {s['start_ms']}-{s['end_ms']} | {s['lang']} | no segment | | MISS")
            continue
        good = best["lang"] == s["lang"]
        ok += good
        lines.append(f"{n + 1} | {s['id']} | {s['start_ms']}-{s['end_ms']} | {s['lang']} | {best['lang']} | {best['lang_prob']:.2f} | {'OK' if good else 'WRONG'}")
    lines += ["", f"result: {ok} of 10 utterances in the right language"]
    Path(a.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    for name, fn in (("prepare", cmd_prepare), ("swap", cmd_swap), ("score", cmd_score), ("control", cmd_control)):
        s = sub.add_parser(name)
        s.add_argument("--scratch", required=True, help="a folder outside the repository")
        s.add_argument("--labels", help="labels.json (default: SCRATCH/labelling/labels.json)")
        s.add_argument("--out", help="evidence file (score, control)")
        s.add_argument("--models", default="en=medium,hi=medium", help="lang=id,... for the engine")
        s.add_argument("--reuse", action="store_true", help="score the batch files an earlier run left in SCRATCH; no decoding")
        s.set_defaults(fn=fn)
    a = p.parse_args()
    if Path(a.scratch).resolve().is_relative_to(REPO):
        raise SystemExit("the scratch folder must be outside the repository (audio and labels never go in git)")
    if a.cmd in ("score", "control") and not a.out:
        raise SystemExit("--out is required")
    a.fn(a)
    return 0


if __name__ == "__main__":
    sys.exit(main())
