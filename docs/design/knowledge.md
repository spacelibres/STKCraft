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

Every `.txt`, `.md`, `.markdown`, `.csv`, `.tsv`, `.json`, `.log`, `.rst`, `.tex`, `.org`, `.adoc`
and `.text` file under that directory is indexed, up to 8 MiB each, 2 000 files, 8 levels deep.
Hidden files are skipped. Nothing is uploaded anywhere: the search is local, and only the passages a
question retrieves are sent to the model.

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
- **Tokens** are lowercased words with the punctuation a number needs (`0.5`, `1e-3`, `24 h`), and
  Chinese, Japanese and Korean text is indexed as characters and character pairs, because those
  languages do not put spaces between words.
- **Ranking** is BM25 (`k1 = 1.2`, `b = 0.75`, the published values). It is deterministic: the same
  library and the same query give the same order on every machine, which is what lets a test assert
  that a specification value is found and a guess is not.
- **Evidence has a floor.** A passage below the threshold is not returned; the search reports `missed`
  instead, and the model is required to say the library does not cover the question. A search that
  always returns something is a search that always lies about its evidence.

## What it deliberately does not do

- **No embeddings.** A vector index needs a model to build, which makes retrieval non-deterministic
  and untestable — and the numbers this project cares about are exact tokens (a frequency, an angle, a
  file name), which lexical search matches better than a vector does.
- **No PDF, DOCX or XLSX reading.** A citation into a PDF is a page and a paragraph, not a line, and
  a parser for each format is a project of its own. Extract the text beside the file — `pdftotext
  spec.pdf spec.txt` — and the library indexes it.
- **No writing.** The library is read-only: the platform never edits, moves or deletes a document.

## A conversation outlives the process

The agent's memory is a **session**: a transcript at
`~/.local/share/stkcraft/sessions/<id>.jsonl`, one JSON object per message, written as the
conversation happens. `stkcraft chat --sessions` lists them and `chat --session <id>` resumes one, so
closing the terminal costs nothing but the typing.

When a session outgrows the context budget (`CONTEXT_BUDGET_CHARS`), the oldest *turns* are dropped —
never half a turn, because a tool result whose call is gone is a request a model API rejects — and the
transcript gains a note saying so. The model therefore knows the conversation has a past it cannot
see, instead of silently believing it began this morning. That is the same shape a harness keeps its
own sessions in.

## Where this is going

- Page-level citations for PDF, and a `.docx`/`.xlsx` text extraction, so a citation can name a page.
- An index cache, so a large library does not re-tokenize on every start.
- A comparison mode: the same question answered from the library and from the engine, with the
  difference shown — the "expert panel" idea taken literally, where the documents and the computation
  check each other.
