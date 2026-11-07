# Remove the local OCR job from CI, 2026-11-07

`AGENTS.md` (CI workflows) requires every workflow to call the shared workflows in
`storytold/craft-common-actions` and forbids a local job. `.github/workflows/ci.yml` had one: an `ocr`
job that installed Tesseract and ran the `#[ignore]`d test `crates/agent/knowledge/tests/ocr_real.rs`.
That job is removed.

## What is lost, and how it is covered for now

The job was the only automated check that the OCR command really reads a scanned page with a real
engine. Without it, that check runs only when a person runs it:

    cargo test -p stkcraft-agent-knowledge --test ocr_real -- --ignored --nocapture

Requirements: Tesseract with the English data, Poppler and Pillow on the path. The plumbing (environment
variables, output bounds) is still covered by unit tests in `ocr.rs` on every run.

## Issue to open in craft-common-actions

Title: Shared CI needs an optional OCR job for apps that read scanned documents

Body: STKCraft verifies its OCR path against a real engine (Tesseract, Poppler, Pillow). The check is an
`#[ignore]`d test, not a unit test. Our workflow must not carry a local job, so we ask for a shared job
or an input to `gate.yml` that installs the OCR stack and runs a named test command. Until it exists, the
check is manual, as described in `docs/plans/2026-11-07-remove-local-ci-job.md`.

## Not done here

- The issue has not been opened: the sandbox cannot reach github.com. Open it from the record above.
- This change is in the working tree only. It needs a version bump and a changelog entry under the rules,
  and a gate run before it is committed.
