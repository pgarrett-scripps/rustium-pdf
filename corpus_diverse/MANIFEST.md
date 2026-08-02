# Extraction corpus

## What this is for, and what it is not for

This checks **parsing**: given a PDF, does this crate recover the right characters, in the right
order, with the right geometry. That is this crate's whole job, so it is tested here.

It does **not** check conversion quality — reading order across columns, block classification,
equation reconstruction, table structure. Those belong to `rustypdf2markdown` and are measured by
its own harness in `eval/`, against LaTeX source. Do not add conversion metrics here: a parser
that scores itself on how good the Markdown looks is measuring somebody else's work, and will be
tuned by the wrong signal.

The line is worth stating precisely, because it is easy to blur:

| question | whose job | measured where |
| --- | --- | --- |
| Did we read the right characters off the page? | this crate | `agree.py`, here |
| Did we put them in the right place? | this crate | `agree.py`, here |
| Is the resulting Markdown any good? | `rustypdf2markdown` | its `eval/`, not here |

## Why these files

The arXiv corpus in `rustypdf2markdown/corpus` is ten papers and **all ten are pdfTeX**. A
producer monoculture hides producer-specific bugs: two word-segmentation defects survived that
corpus and were found within minutes by the files below.

These are bioRxiv preprints, chosen for producer spread rather than subject matter. bioRxiv
accepts Word submissions and renders them, so its output comes from a completely different
population of tools than arXiv's.

| file | creator | producer |
| --- | --- | --- |
| bio00.pdf | Microsoft Word | Microsoft |
| bio01.pdf | Microsoft Word | Microsoft |
| bio02.pdf | Zamzar | Zamzar (online converter) |
| bio03.pdf | Microsoft Word | Microsoft |
| bio04.pdf | Appligent AppendPDF Pro 5.5 | Acrobat Distiller 8.1.0 (Windows) |
| bio05.pdf | Word | — |
| bio06.pdf | Word | macOS Quartz 15.7.7 |
| bio07.pdf | CANVAS X 2020 | Canvas GFX PDF Filter 1.5 |

Provenance is in `sources.txt`. All are open-access preprints under the posting terms bioRxiv
applies; they are test fixtures, not redistributed content, and they are downloaded rather than
committed — 45 MB of third-party files that git history would otherwise keep forever.

## Running it

```sh
./fetch.sh                                             # download the set
cargo build --release --example text
python3 agree.py ../target/release/examples/text *.pdf # needs poppler's pdftotext
```

`agree.py` reports word-bigram recall and word-set Jaccard against `pdftotext -raw`. It is a
disagreement detector, not a correctness oracle: two independently written parsers that agree
closely on a document are very likely both right, and one where they diverge is worth a human
look. That works on any file, including one a user uploaded, which is exactly where no ground
truth exists.

Both an ordering-sensitive metric (bigram) and an ordering-insensitive one (Jaccard) are
reported, so a reading-order difference is not mistaken for a character-level defect. The
reference is `-raw` rather than `-layout` for the same reason: `-layout` sorts geometrically,
and this crate emits content-stream order by design.

Current: mean bigram **0.974** / Jaccard **0.960** on the ten arXiv papers, **0.995** / **0.992**
on these eight.

## Diagnosing a divergence

`examples/fonts.rs` reports per-font unmapped-glyph rates, and is the first thing to run when
text goes missing: a font whose glyphs produce no text is invisible to everything downstream. It
is what found CMEX10 sitting at 83% unmapped, which was losing the display-size operators and
large delimiters out of every equation in the corpus.

## Not covered

Nothing here is scanned, encrypted, right-to-left, CJK, or a slide deck. Scanned pages are an
explicit non-goal — see `Page::is_likely_scanned` — and the rest are simply untested.
