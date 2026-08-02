#!/usr/bin/env python3
"""Agreement between rustium-pdf and an independent extractor, per document.

There is no ground truth for an arbitrary user-uploaded PDF, but there are several mature,
independently-written extractors. Where rustium-pdf and one of them agree on a document's prose,
both are almost certainly right; where they diverge sharply, something is wrong in one of them
and the document is worth a human look. That turns "is this correct?" — unanswerable at scale —
into "does this disagree?", which is cheap and runs on any file.

Reported per document:
  bigram   word-bigram recall of the reference's bigrams found in ours; sensitive to word
           segmentation, which is where producer-specific bugs actually show up
  jaccard  word-set overlap; insensitive to ordering and to segmentation
  len      our character count over the reference's
"""

import re
import subprocess
import sys
from pathlib import Path

def norm_words(text: str) -> list[str]:
    # Ligatures and the dash zoo are extraction-neutral: normalising them keeps the metric on
    # word segmentation rather than on typographic detail.
    text = (
        text.replace("ﬀ", "ff").replace("ﬁ", "fi").replace("ﬂ", "fl")
        .replace("ﬃ", "ffi").replace("ﬄ", "ffl")
        .replace("‐", "-").replace("‑", "-").replace("‒", "-")
        .replace("–", "-").replace("—", "-").replace("−", "-")
        .replace("­", "")
    )
    return re.findall(r"[0-9A-Za-z]+(?:['\-][0-9A-Za-z]+)*", text.lower())


def bigrams(words: list[str]) -> set[tuple[str, str]]:
    return set(zip(words, words[1:]))


def reference(path: Path) -> str:
    # -raw emits content-stream order, which is what this crate emits; -layout would sort
    # geometrically and report a reading-order difference as a character-level defect.
    out = subprocess.run(
        ["pdftotext", "-raw", "-q", str(path), "-"],
        capture_output=True, timeout=180,
    )
    return out.stdout.decode("utf-8", "replace")


def ours(path: Path, dump: Path) -> str:
    out = subprocess.run([str(dump), str(path)], capture_output=True, timeout=300)
    return out.stdout.decode("utf-8", "replace")


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: agree.py <path-to-text-example> <file.pdf> [...]", file=sys.stderr)
        return 2
    dump = Path(sys.argv[1])
    paths = [Path(p) for p in sys.argv[2:]]
    print(f"{'document':<24} {'bigram':>7} {'jaccard':>8} {'len':>6}   verdict")
    print("-" * 66)
    totals = []
    for p in sorted(paths):
        try:
            ref_w = norm_words(reference(p))
            our_w = norm_words(ours(p, dump))
        except Exception as e:  # a crash or timeout is itself the finding
            print(f"{p.name:<24} {'ERROR':>7}   {type(e).__name__}: {e}")
            continue
        if len(ref_w) < 50:
            print(f"{p.name:<24} {'skip':>7}   reference has {len(ref_w)} words")
            continue
        rb, ob = bigrams(ref_w), bigrams(our_w)
        big = len(rb & ob) / len(rb) if rb else 0.0
        jac = len(set(ref_w) & set(our_w)) / len(set(ref_w) | set(our_w))
        ratio = len(our_w) / len(ref_w)
        verdict = "ok" if big >= 0.80 else ("WEAK" if big >= 0.55 else "DIVERGENT")
        print(f"{p.name:<24} {big:>7.3f} {jac:>8.3f} {ratio:>6.2f}   {verdict}")
        totals.append((big, jac))
    if totals:
        n = len(totals)
        print("-" * 66)
        print(f"{'mean':<24} {sum(t[0] for t in totals)/n:>7.3f} "
              f"{sum(t[1] for t in totals)/n:>8.3f}   over {n} documents")
    return 0


if __name__ == "__main__":
    sys.exit(main())
