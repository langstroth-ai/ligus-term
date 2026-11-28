# Changelog

All notable changes to `lt`, the Ligus terminal app (Ligus is short for
ligustica). Versions follow [semver](https://semver.org); releases are on
[GitHub](https://github.com/langstroth-ai/ligus-term/releases).

## Unreleased

### Added

- **Add to my devices takes the desktop's short code.** Type the code
  **Add a computer** shows (`K7Q-4MX`; with your own relay, its address after
  the code) instead of pasting the approval address; `link-to K7Q-4MX` in
  plain mode. A relay without short codes says to paste the address. If the
  relay restarts while lt waits, it asks again (`link-reposted` in plain
  mode). **Link a device: show a code** in the palette shows a code for a new
  device of yours to type; its request goes to your desktop's Inbox. Needs
  the matching Ligus and relay releases.

### Changed

- A turn that isn't this terminal's (an agent on another host, or someone
  else's agent while their devices are away) is never answered here after
  a wait any more; its line says where it waits. Only **Run here this
  time**, on the agent's owner's own device, runs it here.
- This terminal runs only your own agents placed on it. An agent someone
  else placed here doesn't run unless `run_others_agents = true` is in
  `host.toml` in lt's data dir; its sender's chat says so (lt's beat now
  carries the reason, signed by this device).
- An agent whose runtime this terminal doesn't have, or has without its key,
  no longer runs on Claude Code or the workspace's `*` runtime in its
  place: its turn says it can't run here. The `*` runtime still stands in
  for an older agent's runtime id, never for a run spec that names a source
  and model. The chat's default agent keeps its fallback.
- The first run asks one question before anything else: **Is this your
  first device with Ligus?** **Yes, start here** asks your name, then makes
  your workspace (left unnamed, it is "Your name's workspace"), or joins,
  connects or opens the demo. **I already use Ligus on another device**
  goes straight to linking this terminal as one of your devices: no name is
  asked, and nothing on that path can create or join a workspace, so you
  never end up as a second person with your name. `esc` goes back to the
  question; `?` explains the two answers.
- The recovery key screen after creating a workspace can't be closed by
  accident: `esc` or ctrl-c asks **Finish later?** (Keep going / Finish
  later), and finishing later leaves the key unconfirmed.
- lt builds on a pre-release commit of the Ligus crates (the passkey
  hand-off); the release pins the Ligus 0.5 tag.
- A workspace runtime's `--secret-ref` resolves on this terminal only when
  `host.toml` in lt's data dir declares that name for the workspace
  (`[workspaces."<workspace id>"] secrets = ["<name>"]`), as on every Ligus
  host now. Before, any name on the shared log picked
  `LIGUS_WS_SECRET_<name>` from the environment. A runtime that names an
  undeclared secret says which line to add.
- The first start with no `host.toml` in lt's data dir writes one declaring
  the secrets already provisioned in lt's environment
  (`LIGUS_WS_SECRET_<name>`, names only) for each workspace this terminal is
  in, as `ligus agent`, `ligus serve` and `ligus worker` do, so a terminal
  that used those variables keeps working after the upgrade. It never
  touches an existing `host.toml`; later secrets are declared by hand.

### Added

- **Leave workspace** in the palette (`:` then *leave*): lt says in one line
  what happens (the others see you as **asked to leave** until an Admin
  removes you, and this terminal stops syncing the workspace), and that
  leaving isn't revoking: your other devices keep their keys. `y` leaves,
  with your passkey when your account holds one; lt then forgets the
  workspace here. The founder, the last Owner and a terminal linked as one
  of your devices can't leave this way, and lt says why (leave from your
  main desktop, or ask an Admin to remove you).
- **lt follows a rename of itself.** When you rename this terminal's
  computer in My devices on your desktop, lt takes that name live (a toast
  says so), keeps it in its data dir and gives it to its other workspaces,
  instead of reverting it the next time it starts. lt's own `lt
  device-rename` still wins when it is the newer one.

- A message of yours whose turn doesn't run gets one dim line under it:
  "@coder runs on box-1, which is offline. It will answer when box-1 is
  back.", or that the host was removed, or that it can't run there and why
  ("it runs only Ben's agents", "it doesn't have 'codex-pro'"). When the
  agent is yours and this terminal can run it, the line ends with **T Run
  here this time**: `T` (or the chat's `.` menu) runs that one turn here,
  once. A reply that was stopped or cut off reads **Canceled**, one that
  failed **Failed**, with the model and host that ran it.
- On the chat, a reply shows where and when it finished: after anything
  posted while it was written, at its completion's time.

- On a computer linked as one of your devices, a setup step whose change
  the workspace's rules hold for your passkey asks your own desktop or
  phone to confirm it: **Confirm on your desktop or phone:** with what the
  step changes and a code both screens show, and a spinner while it waits.
  Confirmed there with your passkey, the step goes on and writes it; **Not
  me** there writes nothing (**Declined on your other device: nothing was
  written**). `esc` cancels it on both sides; `o` (the browser) and `f` (the
  recovery-words file) still confirm here instead. An expired request (15
  minutes) or no answer in 10 minutes says so, and `enter` asks again. Only
  answers signed by your own devices count. An older relay without the
  approval mailbox keeps the usual passkey sheet.
- A confirmation still waiting when lt quit is offered on the next start:
  a notice, and the first row of the Set up list (`enter` waits for it
  again and writes it once confirmed, `c` cancels it).
- Plain mode prints `presence-handoff` and `presence-handoff-stopped`
  records; `retry ID` asks your desktop or phone again. `lt setup FLOW`
  says on stderr when it waits for your desktop or phone.
- The approvals list names a device of yours asking for your passkey.
- **Add to my devices** brings all your workspaces in one link: when your
  desktop also adds this terminal to your other workspaces (one passkey
  each there), lt checks each one on its own workspace log and asks once
  whether to link as you, naming them all ("In “Acme Web, Beta and Gamma”,
  it acts as “Ana” from then on"). Afterwards
  each extra workspace shows a line saying it linked, or why not and how to
  link it later. A desktop from before this release adds it to the one
  workspace, as before. Needs the Ligus 0.5 crates.
- The Join form says **Looks like you already use Ligus. Link this device
  instead** when this terminal is linked to your account, when you said you
  already use Ligus, or when the checked invite is for a workspace it
  already has; `ctrl-l` switches to linking. It is only a hint: joining
  still works.

## v0.4.1

lt on the Ligus 0.4.1 crates: a device name that stays put, Antigravity's
sign-in checked in setup, linking that never fails silently, and setup
letting an Antigravity agent run local servers.

### Added

- Set up's agent flow can let an Antigravity (`agy`) agent run its own
  local language server: the **May run local servers** choice (off by
  default, the workspace Owner's) writes that agent's signed switch, and a
  test turn of such an agent runs with it. It takes effect on Linux with the
  bubblewrap sandbox; on a Mac it is stored, and setup says it has no effect
  there yet.

### Fixed

- This computer's name no longer changes with the network. lt chose it from
  the Mac's network host name on every launch, so one device showed as
  "lt on Michaels-Air-3" one day and "lt on Michaels-MacBook-Air-3" the next.
  It is now chosen once, the first time lt opens its data dir, from the most
  stable name the OS has (macOS's Sharing name, then its local host name,
  then the host name; on Linux /etc/hostname), kept in the data dir's
  `device-label` file and reused on every launch. Only linking sends it as
  this computer's synced name, and `device-rename` changes it (and is now
  kept for later launches too). An install from before moves once: it keeps
  a name you gave it, or takes the stable name and replaces the
  network-derived one it synced.
- Set up's agent flow can tell whether the Antigravity CLI is signed in. It
  runs `agy models` (read-only, no quota) off the UI thread, with its input
  and output captured, and shows "checking sign-in…" on the harness while it
  runs; a run waits for the answer, so a signed-in `agy` passes instead of
  warning "can't tell from here whether it is signed in". The warning stays
  when the check fails or times out.
- Linking this computer to your devices no longer fails silently.
  - Pasting your desktop's code while its approval is still on the way now
    waits for it. lt says "Waiting for your desktop's approval to arrive…",
    checks again with a growing pause until the code runs out (30 minutes),
    and Esc stops it with nothing linked. Before, it checked once and gave
    up. Between checks this computer stays as it was, so quitting while it
    waits leaves nothing half done. `lt link-redeem` waits the same way
    and says so once.
  - When the link is saved but this computer couldn't confirm it yet (it
    couldn't accept the desktop's certificate, so the workspace doesn't count
    it as your device), lt says "Linked, but not confirmed here yet" with the
    reason, instead of "Linked". Settings shows the same line, R retries, and
    lt also tries again each time it opens. If this computer's name or the
    removal of your earlier member didn't go through, that's said too
    (kept in the log).
  - With the approval address, when the relay stops answering, the screen
    says "Can't reach the relay (why): trying again" until it answers. A
    request that still can't get through ends after its 15 minutes and
    says so, instead of retrying forever with nothing on the screen.

### Dependencies

- Built on the Ligus 0.4.1 crates (`ligus-core` and `ligus-runtime`,
  previously 0.4.0): replies stamped with when they were posted, the chat
  stuck on "this draft is out of date" sending again, the per-agent local
  servers switch, and setup's passkey step pointing to the desktop app.

## v0.4.0

lt on the Ligus 0.4.0 crates: linking by comparing a code, identity model v2
(device names, retiring an earlier member, signing only where the log says
this computer is you), agents' decision cards, proposal archiving and
scratch repositories.

### Added

- Scratch repos. `r` in a chat picks its repository: **New scratch repo**
  first (an empty repository, one empty commit on `main`, made in this
  device's scratch folder, added to the workspace as scratch and checked out
  here; an Admin's, as adding any repository), then the workspace's
  repositories (scratch ones tagged), then **No repository**. A chat in a
  scratch repo shows a small `scratch` tag by its repository, and two days
  before the cleanup (14 days without activity) a calm line: "This scratch
  repo will be removed on ‹date› · K keep it". `K` keeps it as a repo in a
  folder you pick. While `lt` runs it checks hourly for scratch repos it made
  that are due a notice or removal. Set up's "Add a code folder" offers
  "Start with a scratch repo". Plain mode: `repo [scratch|none|N]` and
  `keep [PATH]`.
- Archiving proposals. Archive (`e`, or the row menu) takes a settled
  proposal off the Proposals list for everyone; one settled for the
  workspace's auto-archive days leaves on its own. Hide for me (`H`) takes a
  proposal off this device's list only. `v` lists the archived and hidden
  ones, with Restore and Show again. Neither is counted in what needs you. A
  proposal that still waits on a decision can't be archived. Plain mode:
  `archived`, `archive N`, `restore N`, `hide N`, `show N`.
- Agents' questions (Ligus 0.4 decision cards): a card in the transcript
  where the agent asked, with the agent's words quoted as plain text and
  what the answer does in Ligus's words. `1`-`5` answer, `r` replies, `n`
  says Not now; a grant card shows the exact call and its host, `a` allows
  once after a confirm (and a passkey where the card needs one), `d` denies,
  and a call too large to review offers only Deny. Cards you can't answer
  say who they wait on; settled ones are one line; a chat with a card
  waiting on you shows "needs you". Plain mode's `asks` lists them
  (read-only).
- **Add to my devices with a code match.** Paste the approval address your
  desktop shows (My devices > Add a computer > Copy, `ligusaddr1:`) and the
  request reaches the desktop by itself. Both screens show the same
  8-symbol code; `lt` frames it with "Check your desktop shows the same
  code." and **Yes, they match** / **No**. This replaces the second paste
  and the approving desktop's fingerprint. `lt` acknowledges the desktop's
  offer only after Yes, checks the approval on the workspace log (retrying
  for up to 2 minutes while it syncs), then asks **Link this computer as
  Ana (verified on the workspace log)?**. No at either step grants or links
  nothing, said in words. The request and its fingerprint stay on the
  screen for an older desktop, and a relay without an approval mailbox
  falls back to them. Plain mode: `link-to ADDRESS`, `match yes|no`,
  `link-as yes|no`, `link-cancel`.
- **Approval requests** (palette, People's row menu, plain `approvals`):
  the requests waiting in the workspace's approval mailbox, with their
  kind, label, age and who is checking each, read only ("Approve on your
  desktop or phone").
- **Identity model v2.** Linking accepts the desktop's certificate (rules 3
  counts it only then), names this computer with its device name and
  retires the member this install was before when that is safe; an
  acceptance cut off finishes on the next start. `lt --plain device-rename
  NAME` names this computer as every member sees it. `lt` signs only where
  the workspace log says this computer is the person: a removed person, a
  revoked device, or a workspace it wasn't linked into is read-only, with
  the reason and how to link it there.

### Changed

- This device applies an approved proposal in its checkout of the
  proposal's repository (a scratch repo's, say) even when no checkout is
  bound for the workspace (Hosts, `b`); that binding is still where
  proposals without a repository are applied.

### Dependencies

- Built on the Ligus 0.4.0 crates (`ligus-core` and `ligus-runtime` at tag
  v0.4.0, previously 0.3.1): approval by code through the relay's approval
  mailbox, profiles and synced device names, rules 3 (a device counts only
  once it accepted its approval; 0.3 devices are read-only on such a
  workspace), decision cards, proposal archiving, scratch repositories,
  secrets out of the database with a schema version (a database written by
  a newer Ligus opens read-only), and attachments renewed on the relay.
  Upgrade every device in a workspace.

## v0.3.1

### Added

- Linking from a shell without retyping: `lt --plain link-request` prints
  only the request on stdout (so `| pbcopy` copies exactly the code); the
  fingerprint and the next steps go to stderr. `--copy` copies it, `--json`
  prints `{code, fingerprint, label, expiresAt, newDevice}`.
- `lt --plain link-redeem` with no code, at a terminal, asks for a paste it
  doesn't echo or, on a bare Enter, reads the clipboard (`pbpaste`,
  `wl-paste`, `xclip`, `xsel`, `Get-Clipboard`); `-` reads stdin. It shows
  who approved the code and asks "Is that your desktop?", so
  `--approver-fingerprint` is needed only without a terminal (else exit 4).
  The code never needs to be on the command line.
- The link form's Desktop's code field takes a paste with whatever came
  with it (spaces, line breaks, a whole copied command) and goes straight to
  who approved it.
- A link started and not finished says so: "Linking waiting for your
  desktop's code (started HH:MM; expires HH:MM)" in Settings, once when `lt`
  starts, and from `lt --plain link-status`. Requests no code can answer any
  more are dropped on their own.
- Link to my devices is easy to find: while this terminal is its own member,
  Settings says so on one line, and so does People when another member there
  looks like you (the same name, or a shorter one: "zaz" and "zazulam"). `D`
  in Settings and People links it, both row menus offer it, and the palette
  finds it by "link", "computer", "devices" or "add a computer".
- `c` copies a code meant for pasting elsewhere (the link request, an
  invite, a box's admission or its fingerprint) through the terminal's
  OSC 52 and `pbcopy`, `wl-copy`, `xclip`, `xsel` or `clip.exe`, only when
  pressed; `o` shows the code alone on the screen, with no border in its
  lines, to select it with the mouse. On the link form `ctrl-y` copies while
  you paste the desktop's code. Settings' row menu copies your safety code.

### Fixed

- A linked computer kept posting as its old member when the link was
  redeemed by another `lt` on the same data dir (`lt --plain link-redeem`
  while `lt` was open): once the desktop removed that member, sending failed
  with "role Viewer cannot perform messageAppended". An open `lt` now follows
  a link recorded on disk (the new device key, acting as the person) and
  never signs with the identity the data dir left behind.
- A linked `lt` signs only in the workspaces it was linked into, on every
  write path (posts, agent turns, MCP tools, apply, setup). A refusal where
  the workspace certifies this device for the person but `lt` isn't acting
  as them (a missing `computer-link.json`) says so and how to fix it
  (re-link this computer).
- A linked `lt` shows the person as the workspace's roster has them now, not
  the name the grant carried on the day it was redeemed.
- What this install wrote as its own member and hadn't pushed is pushed
  before it switches to the person's device.
- "role X cannot perform Y" reads as what was refused, why, and the fix.

### Dependencies

- Built on the Ligus 0.3.1 crates (`ligus-core` and `ligus-runtime` at tag
  v0.3.1, previously 0.3.0), a security release: "Allow" on an agent's
  permission request always means once; only the device running an agent's
  turn can ask for a permission, and only the asker closes a question; a
  device acts only for the account it chose; and model API keys are never
  stored in the workspace. Refusals of the role gate now read the same in
  lt and the other Ligus apps.

### Changed

- The key hints always end with `: commands  ? help`, and the first run
  says that `:` opens every command.
- `lt --demo`: the demo's always-on host is called `build-box`.
- `scripts/capture-screenshots.sh` captures every view and the guided
  setup flows in light and dark for the README, langstroth.ai and the docs.

## v0.3.0

### Added

- Guided setup, on the same engine as `ligus setup` and the Ligus
  desktop's wizards: `u` (or the palette: "Set up an agent", "Connect a
  tool", "Add a code folder", "Add a box", "Add a phone", "Invite a
  person", "Set up a relay") opens the Set up list with what is done, and
  `enter` runs a flow one step per screen. Nothing is written before the
  review (`d` makes it a dry run); the run's checks show with their fix
  lines; `esc` during a run stops it between steps and it undoes its own
  writes. Secret fields are masked, held only in zeroized memory and never
  shown or logged. A person's step (pairing a phone, approving a box,
  creating an invite) says where it happens, and offers lt's own Invite
  or Admit a box when it is yours to do. Empty Agents and Hosts views, and
  People with only you, offer the matching flow on `enter`. The list's
  Add a computer row shows whether this computer is one of your devices
  and opens Add to my devices.
- `lt setup [FLOW] [--FIELD VALUE …] [--json] [--dry-run]`: the same flags,
  `ligus.checks.v1` document and exit codes as `ligus setup` (4: a person
  must act). Secrets take `-` or `@FILE`, never the command line. Plain
  mode has a `setup` command; `lt help setup` lists every flow's flags.
- Connect a tool: this device's MCP servers per workspace, with a reach
  per agent and a bearer token kept in the secret store for its origin
  only, offered to turns that run the tool loop here.
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
- **Add to my devices.** Use `lt` as one of your own devices in a workspace
  you are already in on your desktop, instead of joining as a second member
  with your name: a fourth first-run choice, and the palette's **Link this
  computer to my devices** for an install already in use (it asks with a new
  device key, and your desktop can then remove the old member). Your desktop
  approves it under Settings > Profile > My devices > Add a computer with
  your passkey; `lt` confirms the approving desktop's fingerprint before it
  links. Plain mode: `link-request` and `link-redeem CODE
  --approver-fingerprint FP`; `lt help devices`.

### Dependencies

- Built on the Ligus 0.3.0 crates (`ligus-core` and `ligus-runtime` at tag
  v0.3.0, previously 0.2.1): the shared setup engine, Add a computer, the
  `@hive` alias for the default agent and reserved agent names, and relay
  and tool tokens sent only to their own server.

### Changed

- Calmer views, the same pattern as the Ligus app: list rows are one line
  (who or what, then one muted status), so chats, proposals, hosts and
  people no longer take two lines or repeat a label on every row ("DEVICE",
  "Configured").
- Per-row actions moved into a row menu: `.` (or `shift-f10`) lists the
  row's actions, then the view's, then the destructive ones last (they
  still ask first). Each entry shows its key, which still works without the
  menu; a refused one is dimmed with the reason. `enter` stays the row's
  main action. Rebind it as `row_menu` under `[keys.nav]`.
- The key hints show only the view's primary action (invite, add agent,
  start run, approve), the row menu and help; the full list is behind `?`,
  which now opens on what the view is for. The explanations that sat under
  the views (hosts, workflows, proposals, settings, agent usage) moved
  there; empty states are one short line.
- Agents: one list of agents beside the selected agent's detail (runtime,
  host, who can use it, usage, its hand-offs). Adding an agent opens a
  popup of what this device has, instead of a second list under the
  agents, and the runtime table is folded into the change-runtime picker.
- People is one list; its detail pane of role buttons and actions is the
  row menu now. Workflows and proposals drop the action lines from their
  detail, keeping a waiting gate's approve and why you can't vote.
- Narrow terminals stack a view's detail under its list.

### Fixed

- The light theme reads on light terminals. The accent was 2.5:1 on white
  (1.9:1 on the selected row), mentions 2.9:1 and borders 1.3:1, and the
  cream cells of the mark all but vanished; text now keeps 4.5:1 and
  borders 3:1 on the palette's canvas, on white and on Solarized Light
  (and on the selected row), and the mark uses deeper honey tones on a
  light background.
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
