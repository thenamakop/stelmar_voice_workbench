"""Phase 2 parity gate report: wb-engine against whisper-cli on identical chunks (counts only, no transcript text).

    eval/.venv/Scripts/python scripts/gates/parity_report.py --out docs/evidence/phase-2/wer-parity.txt

Reads the finished runs in eval/results (written by `eval/run.py all --only verify-`):
  verify-clichunks-medium-fleurs-<l>   whisper-cli on the chunks wb-engine cuts (role parity)
  verify-wbpar-medium-fleurs-<l>       wb-engine batch (role parity)
and, for information, the context-on English pair and run 1 (each side with its own chunker).

Per language: WER on each side, the difference against the two-sided limit, the number and share of
utterances with identical text, and for every other utterance the number of differing words. Checks that
both sides cut the same chunks, and FAILS if either side's text contains "??" or U+FFFD. Evidence contains
no transcript text (FLEURS is CC BY 4.0 but the evidence stays lean and the rule is the same as for CoSHE).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
RES = REPO / "eval" / "results"
LIMIT = 3.0
LANGS = ("en", "hi", "ur")


def load(name: str) -> tuple[dict, dict]:
    d = RES / name
    summary = json.loads((d / "summary.json").read_text(encoding="utf-8"))
    utts = {}
    for line in (d / "utterances.jsonl").read_text(encoding="utf-8").splitlines():
        if line.strip():
            u = json.loads(line)
            utts[u["id"]] = u
    return summary, utts


def word_diff(a: str, b: str) -> int:
    x, y = a.split(), b.split()
    prev = list(range(len(y) + 1))
    for i, wx in enumerate(x, 1):
        cur = [i]
        for j, wy in enumerate(y, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (wx != wy)))
        prev = cur
    return prev[-1]


def broken(text: str) -> bool:
    return "??" in text or chr(0xFFFD) in text


def wer_pct(summary: dict) -> float:
    return 100 * summary["wer"]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    lines = [
        "Parity of wb-engine and whisper-cli on identical audio (Phase 2, run 2 of 10 Nov 2026)",
        "Method: whisper-cli v1.9.5 decodes the exact chunks wb-engine cuts for each clip (wb-engine chunks --greedy, same chunker, Silero VAD v6.2.0 and 15 s limit as batch), "
        "same model (medium f16), beam 5, best-of 5, temperature fallback, no timestamps, 8 threads, flash attention on; previous-text context off on both sides (role parity: "
        "whisper-cli's --prompt argument is corrupted by Windows' ANSI command line for non-Latin scripts, see run 1 below); the engine is portable AVX2 (GGML_NATIVE=OFF). "
        "Limit: two-sided, 3.0 WER points.",
        "Clips: eval/clips/phase2-verify-fleurs.txt (sha256 7f42da223214dced467d9416e279f82f5aa83ff790def0ddc293f5111db6d3ef), 20 per language.",
        "",
        "language | whisper-cli WER | wb-engine WER | difference | limit | result | identical text | not identical | same chunks | broken text (cli / engine)",
    ]
    ok = True
    details = []
    for lang in LANGS:
        sc, uc = load(f"verify-clichunks-medium-fleurs-{lang}")
        sw, uw = load(f"verify-wbpar-medium-fleurs-{lang}")
        assert set(uc) == set(uw), "the two runs cover different clips"
        diffs, same_chunks = [], 0
        for k in uc:
            d = word_diff(uc[k]["hyp"], uw[k]["hyp"])
            cc = [tuple(c) for c in uc[k]["chunks"]]
            ec = [(s["start_ms"], s["end_ms"]) for s in uw[k]["segments"]]
            same_chunks += cc == ec
            if d:
                diffs.append((k[:8], len(uc[k]["ref"].split()), d, cc == ec))
        n = len(uc)
        ident = n - len(diffs)
        bc = sum(broken(u["hyp"]) for u in uc.values())
        bw = sum(broken(u["hyp"]) for u in uw.values())
        diff = abs(wer_pct(sc) - wer_pct(sw))
        passed = diff <= LIMIT and bc == 0 and bw == 0
        ok &= passed
        lines.append(f"{lang} | {wer_pct(sc):.2f} | {wer_pct(sw):.2f} | {diff:.2f} | {LIMIT} | {'PASS' if passed else 'FAIL'} | {ident} of {n} ({100 * ident / n:.0f}%) | {len(diffs)} | {same_chunks} of {n} | {bc} / {bw}")
        details.append((lang, n, ident, diffs, ident / n < 0.9))
    lines += ["", f"gate (all three within {LIMIT} points, no broken text on either side): {'PASS' if ok else 'FAIL'}", ""]
    lines += [
        "LIMITATION (accepted by the owner, 10 Nov 2026): parity is proven with previous-text context OFF. With context ON it is verified for English only",
        "(20 of 20 utterances identical, below), because no reference tool can carry Hindi or Urdu context on Windows: whisper-cli's --prompt argument is",
        "corrupted by the ANSI command line for non-Latin text. The engine's own context path for Hindi and Urdu (UTF-8 through the library) is therefore not",
        "covered by a parity measurement; it is covered by the engine's own results (the Hinglish gate, and Phase 5, which scores every model through wb-engine batch).",
        "",
    ]
    for lang, n, ident, diffs, low in details:
        lines.append(f"{lang}: utterances whose text is not identical (id prefix, reference words, differing words, same chunks): "
                     + ("none" if not diffs else "; ".join(f"{i} ({r} words, {d} differ, {'same' if s else 'different'} chunks)" for i, r, d, s in diffs)))
        if low:
            lines.append(f"  NOTE: under 90% identical ({100 * ident / n:.0f}%): needs an explanation below.")
    # Information: context on, English.
    try:
        sc, uc = load("verify-clichunks-rolling-fleurs-en")
        sw, uw = load("verify-wb-medium-fleurs-en")
        diffs = [word_diff(uc[k]["hyp"], uw[k]["hyp"]) for k in uc]
        lines += ["", "Information, previous-text context on (English only: Latin text reaches whisper-cli intact), whisper-cli on the engine's chunks against the engine's rolling run: "
                  f"WER {wer_pct(sc):.2f} against {wer_pct(sw):.2f}, difference {abs(wer_pct(sc) - wer_pct(sw)):.2f}; identical text {sum(1 for d in diffs if d == 0)} of {len(diffs)}; differing words per non-identical utterance: {[d for d in diffs if d] or 'none'}"]
    except FileNotFoundError:
        lines += ["", "Information, context on (English): not run."]
    # Run 1.
    lines += ["", "Run 1 (as deployed, information only; not a parity measurement): each side with its own chunker, whisper-cli with the corrupted --prompt for non-Latin text"]
    for lang in LANGS:
        try:
            sc, uc = load(f"verify-cli-medium-fleurs-{lang}")
            sw, uw = load(f"verify-wb-medium-fleurs-{lang}")
        except FileNotFoundError:
            continue
        bc = sum(broken(u["hyp"]) for u in uc.values())
        bw = sum(broken(u["hyp"]) for u in uw.values())
        lines.append(f"  {lang}: whisper-cli {wer_pct(sc):.2f}, wb-engine {wer_pct(sw):.2f}, difference {abs(wer_pct(sc) - wer_pct(sw)):.2f}; utterances with '??' or U+FFFD: cli {bc}, engine {bw}")
    Path(args.out).write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
