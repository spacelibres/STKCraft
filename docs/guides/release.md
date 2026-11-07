# Releasing STKCraft

This guide records the release contract the project is held to and what is actually wired. The
short version: **every version is tagged; only a stable version gets a GitHub Release; packaging
and the shared release workflow are not built yet.**

## Current policy

- `cargo xtask release` reads the workspace version, checks that `CHANGELOG.md` names it, creates
  the annotated `vX.Y.Z` tag, and pushes it.
- A version is **stable** when it is `1.0.0` or later and has no pre-release suffix (`-rc`,
  `-alpha`, `-beta`). Only a stable version prints the `gh release create` command.
- Every 0.x version is a **tag only**. A GitHub Release says "this is the version to use", which a
  0.x version does not promise, and a dev version with both a tag and a Release is the duplication
  the family rule removes.
- The version lives in one place: `[workspace.package] version` in `Cargo.toml`. Every crate
  inherits it with `version.workspace = true`.

## What the shared workflow expects

Sibling Craft apps call `storytold/craft-common-actions/.github/workflows/release.yml@main` from
`.github/workflows/release.yml`. The caller passes:

```yaml
with:
  app-name: stkcraft
  app-title: STKCraft
  craft-fonts-ref: <the suite pin>
  version: ${{ inputs.version }}
secrets: ... # Apple, Windows and Linux signing material
```

The shared workflow builds macOS universal, Windows x64/arm64, Linux, and optionally the web,
FreeBSD, Arch, riscv64 and headless-server outputs. It calls the app's own packaging scripts:

| Path | Called as | Produces |
|---|---|---|
| `packaging/macos/import-cert.sh` | no arguments | imports the Apple certificate into a temporary keychain |
| `packaging/macos/package.sh` | `--arch universal` | the `.app`, `.dmg` and/or archive in `dist/release/` |
| `packaging/windows/package.ps1` | `-Arch x64` / `-Arch arm64` | the Windows installer/archive in `dist/release/` |
| `packaging/linux/package.sh` | no arguments; later `--formats "deb rpm tar"` | AppImage, packages and archive in `dist/release/` |
| `packaging/web/package.sh` | when `has-web` is true | the browser bundle |
| `packaging/freebsd/package.sh` | when `has-freebsd` is true | the FreeBSD archive |

`packaging/env.sh` is the shared value table (`APP_NAME`, `APP_TITLE`, `BUNDLE_ID`,
`APP_SUMMARY`, `stkcraft_version`). It is sourced by those scripts.

The shared workflow also checks that git dependencies come from `github.com/storytold`, that the
lockfile is current, and that release-branch builds have signing secrets. An unsigned release is a
failure on the `release` branch, not a warning.

## What is wired today

- `.github/workflows/ci.yml` calls the shared `gate.yml` on Ubuntu and Windows.
- `.github/workflows/house-rules.yml` checks the shared AGENTS blocks.
- `.github/workflows/packaging-lint.yml` calls the shared packaging lint.
- `packaging/env.sh` holds the shared values.
- `packaging/windows/README.md` keeps the Windows directory present for the packaging-lint glob.

**Not wired:** `.github/workflows/release.yml`, the packaging scripts above, signing secrets and
any installer. `cargo xtask release` therefore tags only; it must not claim a Release exists.

## Cutting a development tag

```sh
cargo xtask version bump
# edit CHANGELOG.md: ## [x.y.z] — <title>
cargo xtask ci
git add -A && git commit -m "F2: ..."
cargo xtask release
```

`cargo xtask release` fails before tagging when the changelog does not name the version or when an
`[Unreleased]` heading is present. Push the release commit through a pull request to `main`; the tag
points at the merged commit.

## Before the first real release

1. Add the packaging scripts and make them produce every artifact the shared workflow uploads.
2. Add `.github/workflows/release.yml` as a caller of the shared workflow, with the signing secrets
   listed in its `secrets:` block.
3. Run the workflow from the `release` branch with the signing material present, and verify the
   signatures (macOS notarization, Windows signing, Linux package signatures).
4. Only then consider `1.0.0` and a GitHub Release.

Until step 3 passes, a 0.x tag is the honest artifact: the source commit, built by anyone who checks
it out.
