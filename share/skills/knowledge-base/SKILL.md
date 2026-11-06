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
3. Quote the passage, then cite it as `path:lines a-b` exactly as the hit reports it. A reader opens
   that file and reads those lines; if they cannot, the citation is a lie.
4. When the search reports that nothing matched, say so in the first sentence. Then, and only then,
   may general knowledge be offered — labelled as general knowledge, never dressed as the library's.
5. If the documents disagree with your general knowledge, the documents win and the answer says which
   document it followed.

## What the passages are

Each hit carries the document's path relative to the library root, the first and last line it came
from, a relevance score, and the text. A paragraph is the unit, so a heading usually arrives with the
paragraph under it — that is deliberate: a citation of a heading alone says nothing.

## Traps

- **A high score is not proof.** Two passages can both look relevant; quote the one that states the
  fact, not the one that mentions the topic.
- **The library is text only.** PDFs and spreadsheets are not read: if a hit is missing for a document
  you know exists, its text has not been extracted into the directory yet, and that is worth saying.
- **Numbers in a document may be stale or conditional.** Quote the condition with the number ("at that
  angle, 1200 km"), because a value without its condition is the kind of answer this project exists to
  avoid.
- **Never re-derive a document's number with the engine** and present it as the document's. If the
  engine computes something close but different, report both and say which is which.
