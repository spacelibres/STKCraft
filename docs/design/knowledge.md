# The knowledge base

STKCraft answers from the user's own documents before it answers from a model's memory. This is not a
feature bolted onto an assistant: it is the reason the platform can be trusted with a mission
timeline. A number that can be traced to a line of the person's own specification is checkable; a
number the model remembers is not.

## Where the documents live

```text
${XDG_CONFIG_HOME:-~/.config}/stkcraft/knowledge/     # Linux
~/Library/Application Support/ai.storyteller.stkcraft/knowledge/   # macOS
%APPDATA%\stkcraft\knowledge\                          # Windows
```

Or point a session anywhere else:

```sh
STKCRAFT_KNOWLEDGE=/path/to/documents cargo run --release -p stkcraft -- chat
```

Every `.txt`, `.md`, `.markdown`, `.csv`, `.tsv`, `.json`, `.log`, `.rst`, `.tex`, `.org`, `.adoc`,
`.text`, `.html`, `.htm` and `.pdf` file under that directory is indexed, up to 8 MiB each for text
and 64 MiB for a PDF, 2 000 files, 8 levels deep. Hidden files are skipped. Nothing is uploaded anywhere: the search is
local, and only the passages a question retrieves are sent to the model.

## Commands

```sh
stkcraft knowledge                       # what the library holds
stkcraft knowledge "maximum off-nadir"   # search it, with citations
```

`stkcraft ask`, `stkcraft chat` and `stkcraft mcp` all search the same library, because they all build
the same session. An MCP client sees the library as two tools — `knowledge.search` and
`knowledge.documents` — and the agent prompt makes the search the first rule.

## How a search works

- **Chunking** is by paragraph, with the line range kept, so a citation is `sensor-spec.md:lines 1-5`
  and a reader can open the file and read exactly those lines. A heading takes the paragraph below it.
  A page of a PDF is chunked on its own, so a citation is `paper.pdf:page 7, lines 3-9` — a paragraph
  does not continue across a page turn in a way a reader can follow.
- **Tokens** are lowercased words with the punctuation a number needs (`0.5`, `1e-3`, `24 h`), and
  Chinese, Japanese and Korean text is indexed as characters and character pairs, because those
  languages do not put spaces between words.
- **Ranking** is BM25 (`k1 = 1.2`, `b = 0.75`, the published values). It is deterministic: the same
  library and the same query give the same order on every machine, which is what lets a test assert
  that a specification value is found and a guess is not.
- **Evidence has a floor.** A passage below the threshold is not returned; the search reports `missed`
  instead, and the model is required to say the library does not cover the question. A search that
  always returns something is a search that always lies about its evidence.

## Reading a PDF

A paper is a PDF, and a knowledge base that cannot open one cannot hold a paper. The reading is done
by the family's shared reader — `craft-pdf` over the patched `hayro` parser and interpreter, the same
pair `pdfcraft` uses, at the same `craft-libs` revision as `craft-ui` — and what
[`stkcraft_agent_knowledge::pdf`](../../crates/agent/knowledge/src/pdf.rs) adds is the part a parser
does not do: **placing text**.

- **A PDF has no lines.** It has glyphs, each painted at a position with a font and a size, in the
  order the generator wrote them — which for a two-column paper is column-interleaved. So the
  interpreter's device is implemented, every glyph arrives with the matrix that placed it, and the
  matrix supplies the position and the type size the assembly needs:
  - a **line** is a run of glyphs whose baselines agree to within a fraction of the type size;
  - a **word break** is the gap left over after the previous glyph's own advance, past a tenth of an
    em — measured against the font's advance, not against the distance between two origins, because an
    `M` advances three times as far as an `i`;
  - a **paragraph break** is a vertical gap past the line pitch, which is what keeps a paragraph whole.
- **A two-column page is read one column at a time.** Fragments are split at gaps wider than an em, and
  the page is ordered column-major when a vertical strip no fragment crosses divides it into groups
  that each hold a real share of the page. A page with no such gutter — the single-column case, and a
  table — is read line by line. The distinction has a test either way, because a page silently read in
  the wrong order is text no human wrote.
- **Work is bounded before it starts**: input size, pages (4 000), glyphs (4 000 000) and characters
  (32 MiB). A document cut short says so: `Document.limited` is set and `stkcraft knowledge` prints
  "read in part".
- **A file that cannot be used is reported.** `Opened::skipped` names it and gives the reason — a scan
  with no text layer, a PDF that will not open, a binary dressed as text — and `stkcraft knowledge`
  prints the list. Indexing nothing while claiming the file is in the library is the worse answer.
- **A scan is read only when an OCR engine is configured.** A scanned page has text in its picture and
  none in its file; with `STKCRAFT_OCR` (the command the page is piped to) the page is recognised and
  `Document.ocr_pages` records how many pages came from OCR. Without it the page is reported as a scan
  rather than guessed at. Running headers and footers remain in the text, because nothing distinguishes
  them from a first line; the citation is still the right page.

## What it deliberately does not do

- **No embeddings.** A vector index needs a model to build, which makes retrieval non-deterministic
  and untestable — and the numbers this project cares about are exact tokens (a frequency, an angle, a
  file name), which lexical search matches better than a vector does.
- **No DOCX or XLSX reading.** Those are zip archives of XML with their own layout model; export to
  text or CSV beside the original and the library indexes that. A PDF *is* read (above), including its
  page numbers.
- **No writing.** The library is read-only: the platform never edits, moves or deletes a document.

## A conversation outlives the process

The agent's memory is a **session**: a transcript at
`sessions/<id>.jsonl` under the data directory (see `storage.md` §4), one JSON object per message, written as the
conversation happens. `stkcraft chat --sessions` lists them and `chat --session <id>` resumes one, so
closing the terminal costs nothing but the typing.

Two limits keep a long session inside the model's context, and they do different things. The
transcript is first trimmed at a turn boundary when it outgrows `MAX_HISTORY` — never half a turn,
because a tool result whose call is gone is a request a model API rejects. Then, when what the model
would receive (messages plus tool schemas) exceeds `CONTEXT_BUDGET_CHARS`, the oldest span is replaced
by a summary the model itself wrote, with a plain note only when that summary call fails. The model
therefore knows the conversation has a past it cannot see, instead of silently believing it began this
morning. That is the same shape a harness keeps its own sessions in.

## Where this is going

- `.docx` and `.xlsx` text extraction, so those documents can be searched and cited.
- A comparison mode: the same question answered from the library and from the engine, with the
  difference shown — the "expert panel" idea taken literally, where the documents and the computation
  check each other.
