# Answering from the user's own documents

A number in this project is either traced to a source or it is not an answer. The user's knowledge
base is the source of record: their requirements, data sheets, thermal memos, review notes. Read it
**before** answering anything factual or numeric.

## Procedure

1. `knowledge.search` with the words a document would use — `maximum off-nadir angle`, not
   `what angle can we point`. Rarer words find the passage; the search ranks by how unusual a term is.
2. If `knowledge.documents` has not been called and the search missed, call it: knowing what the
   library holds is what turns "nothing here" into a useful sentence ("no AOCS data sheet is in the
   library") instead of a shrug.
3. Quote the passage, then cite it exactly as the hit reports it: `path:lines a-b` for a text file, or
   `paper.pdf:page 7, lines 3-9` for a PDF. A reader opens that file at that place and reads the
   sentence; if they cannot, the citation is a lie.
4. When the search reports that nothing matched, say so in the first sentence. Then, and only then,
   may general knowledge be offered — labelled as general knowledge, never dressed as the library's.
5. If the documents disagree with your general knowledge, the documents win and the answer says which
   document it followed.

## What the passages are

Each hit carries the document's path relative to the library root, the first and last line it came
from (and the page, when the document is a PDF), a relevance score, and the text. A paragraph is the
unit, so a heading usually arrives with the paragraph under it — that is deliberate: a citation of a
heading alone says nothing.

## Traps

- **A high score is not proof.** Two passages can both look relevant; quote the one that states the
  fact, not the one that mentions the topic.
- **PDFs are read, but only their text layer.** A scanned paper with no text layer cannot be searched:
  `stkcraft knowledge` lists it under "not in the library" with the reason. Say that rather than
  guessing at what the scan says. Spreadsheets and Word documents are not read either — export them
  as text or CSV beside the original.
- **Numbers in a document may be stale or conditional.** Quote the condition with the number ("at that
  angle, 1200 km"), because a value without its condition is the kind of answer this project exists to
  avoid.
- **Never re-derive a document's number with the engine** and present it as the document's. If the
  engine computes something close but different, report both and say which is which.
