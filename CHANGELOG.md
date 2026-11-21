# Changelog

All notable changes to `lt`, the Ligus terminal app (Ligus is short for
ligustica). Versions follow [semver](https://semver.org); releases are on
[GitHub](https://github.com/langstroth-ai/ligus-term/releases).

## v0.1.1

### Added

- The Agents tab finds what this device can run agents on, the way the
  desktop app does at onboarding: Claude Code, Codex, the Copilot and
  Antigravity CLIs on `PATH` (version and sign-in), a local Ollama server's
  models, and API keys in the environment. "Available on this device" lists
  the ones no agent uses yet, with their kind, model and whether they are
  ready; `N` (or `enter`) adds one as an agent that is yours, "Only me" and
  runs on this device. With nothing found, it says what to install. Plain
  mode: `available` and `add-agent N [NAME]`. Detection only runs
  `--version`, never opens a keychain and never reads a key's value.

### Fixed

- An invite code is on the relay before `lt` shows it. Creating an invite
  or admitting a box used to return the code before the relay had the
  issuance, so a quick joiner could race the sync loop, and a code printed
  just before `lt` exits (plain mode) could not be redeemed. Both now push
  the workspace first.

### Tests

- New test: an invite is on the relay before its code is shown.
- The deferred-claim fallback tests run on a hand-driven clock instead of a
  20-second wall-clock wait, and the start-loops test checks structurally
  that the loops start while another thread holds the write lock, with no
  timing bound. Test waits are capped at 60 seconds.

### Release and install

- Prebuilt binaries for macOS (arm64, x86_64) and Linux (arm64, x86_64,
  static musl) on each GitHub release, each with a `.sha256` checksum. The
  macOS binaries are signed and notarized as `ai.ligus.term` once the
  signing secrets are configured.
- A Homebrew formula: `brew install langstroth-ai/ligus/lt`.
- A release pipeline: `cut-release.yml` tags a commit that passed CI and
  runs `release.yml` (draft, builds, checks, publish as a prerelease,
  formula). CI builds all four release targets and has one aggregate
  `ci-ok` check.

## v0.1.0

The first tagged version: a full Ligus client in the terminal, on the Ligus
crates at the ligus repo's `v0.1.0` tag. Install from source only.
