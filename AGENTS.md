# AGENTS.md: working on lt

How an agent (or a person) builds, tests and runs `lt`. Read
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) before changing structure.

## Build

```sh
export CARGO_TARGET_DIR="$PWD/target"   # keep build output in the repo (gitignored)
cargo build --locked
```

The Ligus crates are a path dependency on a local checkout of the ligus
repo at `../f-term-crates` (branch `feat/runtime-for-clients`) until they
are tagged; see the note in `Cargo.toml`. At release they become git
dependencies pinned to tag `v0.1.0`.

Rust 1.91 or newer, edition 2024. The binary is `target/debug/lt`.

## Test

Run all of these before committing. CI runs the same on Linux and macOS
(`.github/workflows/ci.yml`):

```sh
cargo fmt --check          # (--all also walks the path dependency)
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo deny check            # licenses, advisories, sources (if installed)
```

What the tests cover:

| File | What |
|---|---|
| `tests/screens.rs` | Every screen rendered into a ratatui `TestBackend` and compared as text (insta snapshots in `tests/snapshots/`) |
| `tests/reducer.rs` | The reducer: messages in, state and effects out, no backend |
| `tests/keymap.rs` | Default bindings, contexts, typing rules, config overrides and errors |
| `tests/cli.rs` | The `lt` binary: `--version`, plain mode on a pipe, refusals, the data dir |
| `tests/real_backend.rs` | The real backend: two devices through the in-process test relay (create, invite, join, chat, streamed replies through both dispatch gates, proposals, vote, apply on host, people, hosts, lockdown), and a device reopened from disk |
| `src/**` unit tests | Fake backend behavior, the presence loopback hand-off, the recovery-words file, data-dir rules, text editing, plain-mode scripts |

When a UI change alters a screen, the snapshot test fails and shows the
diff. If the new screen is right, accept it and refresh the captures in
`docs/screens/`:

```sh
INSTA_UPDATE=always LT_WRITE_SCREENS=1 cargo test --test screens
git diff tests/snapshots docs/screens   # read it before committing
```

## Run

```sh
cargo run -- --demo          # sample workspaces
LT_DATA_DIR=$(mktemp -d) LIGUS_SECRET_BACKEND=file cargo run   # a throwaway real device
cargo run -- --demo --plain  # line mode; type help
```

An agent can't drive the full-screen UI, so check behavior through plain
mode, which uses the same backend:

```sh
printf 'chats\nopen 1\nsay @coder fix it\nwait 5\nproposals\nquit\n' | cargo run -q -- --demo --plain
```

Or add a snapshot test that drives the reducer with keys (see
`tests/common/mod.rs`, `Harness`).

## Rules

- **Never the owner's data.** UI tests run against `backend::fake`. Tests
  of the real backend (`tests/real_backend.rs`) use temporary data dirs,
  `LIGUS_SECRET_BACKEND=file` (never the login keychain), the crates'
  in-process test relay and a software passkey. Never point `lt` at a real
  data dir from a test, never run a real `ligus` binary without
  `LIGUS_DATA_DIR` set to a temp dir and `LIGUS_SECRET_BACKEND=file`, and
  never cause a macOS permission prompt.
- **No authorization in the UI.** Whether an action is allowed comes from the
  backend as `Allowed::No(reason)`. The UI shows the reason and disables the
  action; it never re-derives rank rules.
- **The reducer stays pure.** `app::update` changes state and returns
  `Effect`s. It never awaits, never calls the backend, never draws. Rendering
  (`ui::*`) only reads state.
- **Keys go through the keymap.** Add an action to `keymap::BINDINGS` (its
  context, config name, help text and default keys), handle it in
  `app::update`, and the help overlay, `--print-config` and config overrides
  pick it up.
- **Every screen has a snapshot.** A new screen or overlay gets a test in
  `tests/screens.rs` and a line in `docs/screens/README.md`.
- **Plain mode follows the TUI.** A new view or action gets a plain command
  and a record format.

## Dependencies

Direct dependencies are pinned to releases that were at least 7 days old
when chosen, and so is everything in `Cargo.lock`. After adding or updating
a dependency, check:

```sh
python3 scripts/check-dep-age.py 7    # lists anything newer than 7 days
cargo update -p NAME@NEW --precise OLD
```

## Commits

Small, logical commits. Don't push; the GitHub repo
(`langstroth-ai/ligus-term`) is created at release.
