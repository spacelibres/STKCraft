# shellcheck shell=bash
#
# Shared values for the packaging scripts.
#
# Sourced by packaging/macos/package.sh, packaging/linux/package.sh and the web build. The
# display name is what users see; every file name, binary and identifier stays lowercase.
#
# Bash, not POSIX sh, because `stkcraft_version` is exported as a function (`export -f`) for the
# per-platform scripts to call. Shellcheck is told so explicitly: without a directive it reports
# SC2148 ("shell type unknown"), which was the first CI failure this repository ever had — the
# packaging workflow had never run, because it was gated on a branch that did not exist.

# The lowercase program name, used for binaries, file names and folders.
APP_NAME=stkcraft

# The display name, used in window titles, menus, installers and release names. Always
# written in PascalCase, like the sibling Craft apps.
APP_TITLE=STKCraft

# Reverse-DNS identifier: `ai.storyteller.<app>`.
BUNDLE_ID=ai.storyteller.stkcraft

# One-line description, used by the Linux metainfo and the About window.
APP_SUMMARY="Open-source space mission analysis and visualisation"

export APP_NAME APP_TITLE BUNDLE_ID APP_SUMMARY

# The version lives in exactly one place: `[workspace.package] version` in Cargo.toml.
# Read it here rather than duplicating it.
stkcraft_version() {
    sed -n 's/^version = "\(.*\)"/\1/p' "$(dirname "$0")/../Cargo.toml" | head -n 1
}

export -f stkcraft_version 2>/dev/null || true
