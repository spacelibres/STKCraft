# Credits

`people.toml` is the source a person edits: one `[people.<github-username>]` entry, with the name
that person chose. **Never add, edit or guess someone else's entry**; ask them once.

`contributors.json` is generated, and it is not committed. At release time the app calls
`storytold/craft-common-actions`' `refresh-contributors.yml`, which checks out the full history,
runs the shared generator with a read-only token, and uploads the snapshot as a release artifact;
the source-build jobs install that artifact before packaging. There is no local generator to keep
in step, and a hand-edited `contributors.json` is a file the next release overwrites.

The generator credits people from linked Git identities and merged pull requests, and recognizes
coding agents by their exact Git email or GitHub account, never by a name. That is why the
repository keeps the hand-written file small and the generated file absent.
