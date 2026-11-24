# Changelog

All notable changes to `lt`, the Ligus terminal app (Ligus is short for
ligustica). Versions follow [semver](https://semver.org); releases are on
[GitHub](https://github.com/langstroth-ai/ligus-term/releases).

## Unreleased

### Added

- Every Ligus palette: Pollen, Studio, Harbor, Meadow, Slate and Obsidian,
  each for a light and a dark terminal. `theme = "harbor"` picks one and
  `appearance = "auto" | "light" | "dark"` the background (also `--theme`
  and `--appearance`); `theme = "auto" | "dark" | "light" | "mono"` work as
  before. Settings has Theme and Appearance rows, and the palette lists
  every theme, showing each as you move onto it.
- Your own themes: `themes/NAME.toml` (or `.json`) next to `config.toml`, in
  the shared Ligus theme format the desktop imports and exports. Colours
  only, checked strictly; errors name their line, low contrast is a
  warning. `lt themes list` and `lt themes check FILE` (each with `--json`),
  `lt help themes`, and a `themes.valid` check in `lt doctor`.

### Fixed

- The light theme reads on light terminals. The accent was 2.5:1 on white
  (1.9:1 on the selected row), mentions 2.9:1 and borders 1.3:1, and the
  cream cells of the mark all but vanished; text now keeps 4.5:1 and borders 3:1 on the palette's canvas,
  on white and on Solarized Light (and on the selected row), and the mark
  uses deeper honey tones on a light background.
- `appearance = auto` asks the terminal for its background before reading
  `COLORFGBG`, which a shell can inherit from another terminal (over ssh,
  in tmux), and reads Solarized Light's `COLORFGBG` (`11;15`) as light.

## v0.2.1

### Changed

- Built on the Ligus 0.2.1 crates (previously 0.1.0). Writes from sync,
  the desktop and lt landing in one chat at once are no longer lost or
  refused with "database is locked"; secret redaction no longer stalls
  while many secrets are registered, and fails closed if its matcher
  can't be built.

### Security

- A chat's history is kept per workspace: a member of another workspace
  this device also syncs who knows a chat's id can no longer hide, re-home
  or disturb it. lt reads every chat in its own workspace.
- The first start on this version upgrades the local database once (the
  same upgrade as Ligus 0.2.1); it needs about 2.5 times the database's
  size in free disk space and says how much to free otherwise.

## v0.2.0

### Breaking

- Exit codes changed, which can break scripts that test for a specific
  code: a one-shot command exits with its first error's code instead of
  always 1 (an unknown chat is 5, a refusal 6), and a data dir or device
  lt can't use at start-up exits 3 instead of 2. Scripts that only test
  for non-zero are unaffected. See Changed below for the full table.

### Added

- A command palette: `:` (or `ctrl-p`, `ctrl-k`, also from the composer)
  opens one fuzzy list of the focused view's actions (naming what they act
  on: "Approve “Fix the flaky test”"), every action available anywhere,
  every view, and the workspace's other workspaces, chats, proposals, runs
  and agents. Each action shows its key. `enter` runs it or goes to the
  thing and selects it; `esc` closes. Context actions only show where they
  apply, so nothing acts on a selection you can't see.
- The first run has a fourth choice, "Look around first (the demo,
  nothing saved)", which restarts as `lt --demo`. A new workspace's empty
  chat shows a getting-started list from its real state (agents, people,
  boxes, runs) with the keys for each step and the runtimes found on this
  device, in place of the welcome, until every step is done.
- Needs-you signals: a chat that newly needs you (an agent's question, a
  proposal waiting on your vote) rings the bell, and while the terminal is
  in the background lt also sends one desktop notification (OSC 9; through
  tmux's passthrough in tmux), batched when several arrive together. A
  turn you asked for that finishes or fails while you're away does the
  same. What already waited when the workspace opened stays quiet. The
  Settings toggle ("Notify me when I'm @mentioned or needed") covers all of
  it.
- `--reduce-motion` (or `LT_REDUCE_MOTION=1`): spinners stand still; a
  running item shows a fixed glyph and its elapsed time.
- `/` finds in any list (chats, proposals, agents, hosts, people,
  notifications, workflows and runs, memory): the selection jumps to the
  first match as you type, `↑`/`↓` go to the previous or next one, `enter`
  keeps it and `esc` goes back to where you were. The status bar shows
  what's typed and how many rows match.
- View history: `ctrl-o` (or `alt-left`) goes back to the view you were
  in, `alt-right` forward again.
- `D` in Chats shows the whole transcript as plain text in `$PAGER`, or,
  with no pager set, leaves it in the terminal's scrollback.
- Quitting prints one dim line when something is still going: runs still
  moving (and who drives them), proposals waiting on your vote, agents'
  open questions.
- Focus mode: `ctrl-f` (or the palette) hides the top bar and the key
  hints, leaving the work; one quiet `◆ N` badge on the top border shows
  while something needs you (unread mentions, votes, agents' questions).
  `ctrl-f` again brings them back.
- `lt doctor` (`--json` for a `ligus.checks.v1` report, the shape every
  Ligus doctor shares): the data dir, the config, the secret store's kind,
  `LT_RELAY_URL`, the workspaces and their relays, the agent runtimes on
  `PATH` and the terminal, without opening the device (no keychain prompt,
  no network). `next` names `langstroth-relay check --url …` per relay and
  `ligus doctor` when the CLI is here. Exit 1 when a check fails.
- Tab completion and a man page: `lt completions bash|zsh|fish` prints a
  completion script (flags, commands, help topics, themes, and workspace
  names after `--workspace` and `use`), and `lt man` prints the man page.
  Neither opens a device; workspace names come from the data dir's
  workspace list alone, so completion never touches the secret store.
  `lt help completions` says where to put them. The Homebrew formula
  installs both.
- The mouse: click a tab, a chat or a proposal, or the transcript or
  composer to focus it; the wheel scrolls lists, the transcript and the
  diff, and moves through overlays. On by default; `mouse = false` in the
  config turns it off. Selecting text takes shift-drag while lt has the
  mouse, and the invite overlay says so.
- `--ascii` (or `LT_ASCII=1`): ASCII only, with `+-|` borders, the status
  glyphs `*` `<>` `~` `ok` `x` `=`, and no block-art mark, for limited
  fonts and braille displays.
- Switch the theme in the app: Settings has a Theme row (`enter`: dark,
  light, mono), and the palette has "Switch the theme". It lasts the
  session; `theme = …` in the config file keeps one.
- `space`, then a pause, shows a sheet of the single keys you can press
  here: the view's actions, the views, and what works anywhere. The next
  key acts as it always does; `esc` or `space` put the sheet away. Pressing
  the key without pausing shows nothing.
- The composer remembers what you sent: `↑` on an empty draft recalls
  your messages in this chat (then what you sent elsewhere this session),
  newest first, and `↓` comes back; editing a recalled message keeps it as
  the draft. `ctrl-r` searches them. `ctrl-g` opens the draft in
  `$VISUAL` (else `$EDITOR`, else `vi`) and puts what you save back in the
  composer; the draft goes through a private temporary file (0600) that is
  deleted afterwards. `i` still enters the composer.
- The transcript reads better: minimal Markdown (bold, inline code,
  headings, lists, fenced code in a box with its language); one line per
  tool call with how it ended and how long it took, collapsed, and `o`
  shows the first lines each returned; "· on sff" after an agent's runtime
  says where its turn runs; a "→ proposal" line for each proposal the chat
  produced (`◆` and "p to review" when it waits on your vote); a streaming
  reply shows one spinner cell and its elapsed time instead of a
  warn-coloured "generating".
- `ctrl-x` stops the agent's turn in the open chat when it runs on this
  device (from the composer too); otherwise it says the turn's host stops
  it. Plain mode has `stop`, which prints a `stopped` record.
- Chats show where each one stands, with one glyph before the title: `⠹`
  working, `◆` needs you (an agent's open question or a proposal waiting
  on your vote), `⧗` queued, `✓` done since you last looked, `✕` failed and
  `‖` paused by a lockdown (it was `⊘`). The states and their precedence
  are the desktop's, so both agree on a chat. Plain mode's `chats` adds a
  `needs-you` flag.
- A messages log: `!` lists this session's notices and errors in full,
  newest first, so a toast that went by (or was cut to fit) can be read
  again.

### Changed

- Exit codes follow the table every Ligus tool shares: 0 done, 1 failed,
  2 usage, 3 a precondition you can fix, 4 a person must act, 5 not found,
  6 refused by workspace rules, 7 offline. A one-shot command now exits
  with its first error's code (it was always 1; for example an unknown
  chat is 5, a refusal 6), and a data dir or device lt can't use at
  start-up is 3 (it was 2). Scripts that test for non-zero are unaffected.
- A one-shot command also writes its error to stderr as `lt: <what>`; the
  `error` record stays on stdout. Start-up errors add a `fix:` line.
- Quieter notifications: a mention in the chat you're looking at is
  already on screen, so it doesn't ring, toast or count in the badge; a
  burst of notifications is one toast ("3 new notifications (n reads
  them)") and the messages log keeps each.
- `esc` in a view goes back to the view you came from, instead of always
  to Chats.
- Colours come from the shared Ligus tokens, the same values the desktop
  and the phone use, so the three can't drift apart: each colour at 24-bit,
  its own 256-colour index and a 16-colour slot. In 16 colours the accent
  and focus are bold rather than a hue, so yellow means a warning only.
  Text, dim, warn, danger and the rest shift slightly to the shared values.
- `theme = auto` asks the terminal for its background (OSC 11, at most
  100 ms at start, only in the full-screen app) when `COLORFGBG` doesn't
  say, instead of assuming dark.
- The chat view fits the terminal: under 100 columns the chat list shows
  only while it has the keyboard (`esc` from the composer, or the new
  `ctrl-l`), and the transcript has the width the rest of the time, its
  title saying where the chat sits ("· 1/3"). From 160 columns a third
  column shows the open chat's work: who is working, its proposals and its
  runs. Transcript lines stop at 100 columns, centred. Below 60×20 one line
  asks for a bigger terminal instead of a broken layout (a passkey prompt
  still shows).
- Settings toggles with `enter` or `t`; `space` is the key sheet's
  everywhere now (rebind `toggle` in `[keys.settings]` to have it back).
- The status bar has three zones: the keys on the left, a notice in the
  centre, and the sync state on the right in words (`⇅ relay ok`,
  `⚠ offline`, `⚠ sync off`, `peer to peer`, `this device only`) with how
  many agents and runs are working ("2 running"). Warnings use the warn
  colour, not danger's red. The device's name moved out of the status bar
  (Settings shows it); the demo still says "demo".
- Toasts no longer stack in the bottom-right corner over pane borders and
  the composer: one shows at a time, in the status bar, and a new one
  replaces it. Errors stay twice as long.

## v0.1.3

### Added

- The help overlay (`?`) searches: `/` narrows it to the keys whose key,
  description or view match what you type; `enter` keeps the filter and
  gives the arrows back, `esc` clears it, a second `esc` closes the help.
  The sections for what has focus come first and are marked "here".

### Changed

- Proposals shows the selected proposal's diff as soon as you select it,
  instead of "enter to view the diff". Moving through the list loads only
  the diff the selection rests on, so holding a key stays light; `enter`
  still moves into the diff to scroll it.
- Warnings have their own colour in the dark theme: an amber-orange off the
  honey accent (the desktop's warn), so an open proposal or a warning no
  longer looks like the accent or the focused pane. The light theme uses
  the desktop's light amber-orange, and with 16 colours warn is the
  terminal's yellow.
- The demo (`lt --demo`) shows an ordinary software team: a "Northwind API"
  workspace with chats such as "Fix flaky checkout test", "Migrate auth to
  passkeys" and "Weekly release notes", a checkout-timeout proposal, and
  the "Implement, review, ship" and "Weekly security digest" workflows,
  plus a small "docs-site" workspace. It has the same agents, runtimes,
  hosts, proposals, runs, people, memory and notifications as before, in
  the same states. The help examples, screen captures, README images and
  demo GIF use the same names.

### Fixed

- At 80×24 a locked-down chat's banner cut off the key that clears the
  lockdown; it now comes first and the reasons end in "…". An empty chat's
  welcome wraps instead of losing its end, and the composer's hints drop
  whole parts rather than stopping mid-word.

### Tests

- Every screen, including the overlays, sheets and first-run steps, is
  snapshot at 80×24 and 200×50 as well as 110×32.

## v0.1.2

### Added

- `lt` documents itself. `lt --help` lists every flag, every environment
  variable `lt` reads (with its default), the help topics, the plain-mode
  commands and the exit codes. `lt help TOPIC` covers `quickstart`, `keys`
  (built from the keymap), `plain`, `agents`, `kubernetes`,
  `troubleshooting` and `env`; `lt help COMMAND` explains one plain-mode
  command and the records it prints; `lt help --FLAG` one flag. `lt help`
  needs no device. In plain mode, `help COMMAND` and `help TOPIC` print the
  same text as `help` records.
- `--json`: plain mode writes each record as one JSON object per line, with
  named fields. It implies `--plain`, and a one-shot listing command also
  takes it at the end (`lt --plain agents --json`). Counts (`n`, `added`,
  `removed`, `turns`, `uncosted`, `cost_usd`, `progress`, `open`, `port` and
  a passkey prompt's `id`) are JSON numbers, `null` when missing, and the
  two toggle settings are booleans; everything else is a string.
- `lt COMMAND` runs one command without `--plain` (`lt agents`, `lt runs
  --json`). A mistyped command is refused before any data dir is opened.
- `--name NAME` names a new device's identity, so a one-shot `create`,
  `join` or `connect` works on a fresh device (`lt --name Ada join CODE`).
- Passkeys with no browser on this device: `LT_NO_BROWSER=1` shows the
  passkey page's address on your terminal (below the prompt, or on the
  controlling terminal in plain mode; never on stdout) for a browser on
  another device that reaches this one's loopback over `ssh -L`, and
  `LT_PRESENCE_PORT` fixes that port so it can be forwarded ahead of time.
  `lt help troubleshooting` explains it, and the passkey-free alternative:
  create the workspace where a browser is, then `join` on the headless box.
- The status bar shows how the workspace syncs: `⇅ synced`, `⚠ sync
  failed`, `⚠ sync off`, `peer to peer` or `this device only`. Failures of
  background work (a turn, a workflow, an apply) appear as error toasts
  instead of going unseen.
- Terminals without 24-bit colour get the nearest of 256 colours, or the 16
  ANSI colours on a basic `TERM` such as `linux` (so the terminal's own
  palette applies), chosen from `COLORTERM` and `TERM`;
  `LT_COLOR_DEPTH=truecolor|256|16` overrides it. Settings shows the depth.

### Changed

- lt starts with the cursor in the open chat's composer, so you can type at
  once; `esc` leaves it for the keys (`f1` opens help from the composer).
- `q` (and `ctrl-c`) asks before quitting while this device drives a
  workflow run or an agent's turn; a second `ctrl-c` quits anyway.
- `X` (cancel a run) and `R` (reject a proposal or a run's gate) ask first:
  `y` goes ahead, any other key backs out.
- Notifications marks all read with `M` instead of `R`, so `R` only ever
  rejects. Rebound keys in your config keep working; a config that set
  `mark_all_read` keeps its keys.
- The diff viewer moves between hunks with `]` and `[` only: `n` and `N`
  now open Notifications and a new chat there too, as everywhere else.
- Help lists the keys of what has focus first, scrolls to its end (`g`,
  `G`, page keys) without a dead zone, and shows where you are.

### Fixed

- At 80×24 and smaller the top bar cut off People, Notifications (with its
  unread badge) and Settings. It now shortens the labels, then shows one
  workspace tile and the view digits, then only the current view, and
  always keeps the unread badge.
- The status bar ran into the device label (`? helplt on this terminal`).
  Hints now drop from the right, the help key always stays, and the right
  side keeps a two-column gap.
- The Agents view showed two of four agents and one of four runtimes at
  80×24. Its panes now get room by priority (agents, what this device could
  add, runtimes, usage, hand-offs); a pane that doesn't fit is left out and
  named in the title. Wide terminals put usage and hand-offs side by side.
- Workflow names were cut to `▸ Im…` while the chat title kept its room;
  the name comes first now, for runs too.
- The usage pane ran its label into the numbers (`Today 11-20Mara`).
- A run's header said "started by Mara now ago"; relative times in
  sentences now read "just now" and "5m ago".

### Tests

- Every main view is snapshotted at 80×24, 60×20 and 200×50 as well as
  110×32, checking that no view drops out of the top bar, the unread badge
  survives and the status bar keeps its help key; the 80×24 captures are in
  `docs/screens/`.
- New tests: quitting asks while a run is driven here, confirmations for
  cancel and reject, no view key hides a global key, help scrolls within
  bounds, the passkey page's address never reaches stdout, the no-browser
  hand-off on a fixed port, colour-depth detection and mapping, and `--json`
  numbers.
- New `tests/help.rs`: every flag is in `lt --help` and has its own page,
  every plain-mode command has help, every record keyword has JSON field
  names, every environment variable read in the source is documented, the
  key table matches `keymap.rs` row for row, and `--json` output parses.

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
- `lt` builds for aarch64 Linux (musl): the Ligus crates are re-pinned to
  the ligus repo's re-cut `v0.1.0` tag, which names `kexec_file_load` by
  number where the libc crate lacks it.

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
