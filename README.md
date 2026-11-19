# lt: Ligus in the terminal

`lt` is a full Ligus client for the terminal. (Ligus is short for
*ligustica*, the Italian honeybee.) It is a device in its own right: it has
its own keys, joins workspaces with an invite, signs its events like the
desktop app does, and works without a server. Everything the desktop shows
has a terminal counterpart: chats with streaming agent replies, proposals
and their diffs, agents, hosts, people and roles, lockdowns and
notifications. It also has a plain line mode for screen readers and
scripts.

> **Status: early.** `lt` runs on the Ligus crates: it creates and joins
> workspaces, syncs through a relay, signs its events, answers the turns
> it owns, and applies approved proposals on its own checkout. `lt --demo`
> runs the same UI on an in-memory sample instead. What isn't wired yet is
> listed in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#what-is-wired).

## Install

Release binaries and package-manager installs will be listed here.

From source (Rust 1.91 or newer):

```sh
cargo install --locked --path .
lt --version
```

## Use

```sh
lt                 # first run: your name, then create a workspace or paste an invite
LT_RELAY_URL=https://relay.example lt   # new workspaces sync through this relay
lt --demo          # explore with sample workspaces (keeps nothing on disk)
lt --plain         # line mode for screen readers and scripts
lt --print-config  # a config file with every default
```

On first run `lt` asks for your name, then lets you **create a workspace**
(no server needed) or **join one** by pasting an invite code. Creating one
asks for a passkey and then shows your 24-word **recovery key** once: write
it down.

A workspace created without a relay stays on this device. To invite others,
start `lt` with `LT_RELAY_URL` set to a relay before creating it: invites
carry the relay, and joining needs one.

### Screens

Press the view's key from anywhere outside a text field:

| Key | View | What's there |
|---|---|---|
| `c` | Chats | Chat list, transcript with streaming agent replies, tool calls, who's typing, the composer with @mention completion, the lockdown bar |
| `p` | Proposals | Status, votes, Approve/Reject, "Apply on ‹host›", and a diff viewer that scrolls and jumps between hunks and files |
| `a` | Agents | Workspace agents (runtime, role, which host runs them) and the runtimes they can use |
| `h` | Hosts | This device and the workers, online or last seen, and the agents on each |
| `m` | People | Members, roles, founder and service badges. Actions the rank rules refuse are crossed out, with the reason |
| `n` | Notifications | @mentions of you; Enter jumps to the chat |
| `s` | Settings | You, notifications, this device (data dir, secret store), theme and config |
| `w` | Workspaces | The switcher; `+` creates or joins another |
| `?` | Help | Every keybinding, as currently configured |

Text captures of every screen are in [docs/screens](docs/screens/README.md).
Screenshots will be added later.

### Keybindings

These are the defaults. All of them can be rebound (see Configure below).

| Where | Keys |
|---|---|
| Anywhere | `q` / `ctrl-c` quit · `?` help · `esc` back · `w` workspaces · `+` add workspace · `tab` / `shift-tab` next or previous pane |
| Lists | `↑↓` or `j k` move · `pgup pgdn` · `g G` first and last · `enter` open |
| Chats | `i` write · `N` new chat · `L` lock the chat down · `U` clear the lockdown |
| Composer | `enter` send · `alt-enter` / `ctrl-j` new line (`shift-enter` too, in terminals with the kitty keyboard protocol) · `@` mention, then `tab` or `enter` to accept · `esc` leave (the draft is kept) |
| Proposals | `A` approve · `R` reject · `O` apply on a host · `W` withdraw the apply request |
| Diff viewer | `]` / `[` (or `n` / `N`) next and previous hunk · `}` / `{` next and previous file |
| People | `r` change role · `X` remove · `I` invite |
| Agents | `r` change runtime |
| Passkey prompt | `enter` open the browser · `f` use the recovery-words file · `esc` cancel |

While the composer or another text field has focus, letters type. Only
chords with `ctrl` or `alt` (and function keys) reach the global bindings.

### Passkeys in a terminal

Changes to who's in a workspace, and some votes, need your passkey. `lt`
uses the same hand-off the desktop does: it opens your browser to the
workspace's passkey page (`ligus.ai/presence` by default) and waits for the
result on a one-time loopback address. The sheet shows what you're
authorizing and a digest to compare with the browser.

No browser (an SSH session, a headless box)? Press `f` and give the path to
a file holding your 24 recovery words, the same file the `ligus` CLI's
`--recovery-words-file` flag takes. It must be readable only by you (`chmod 600`).

### Plain mode

`lt --plain` (also used automatically when stdin or stdout isn't a
terminal) reads one command per line and writes one record per line: a
keyword, tab-separated fields, text last. New messages, typing, proposals
and passkey prompts print as they happen, so a screen reader announces them.

```sh
printf 'chats\nopen 1\nsay @coder fix the threshold\nwait\nproposals\n' | lt --demo --plain
```

Type `help` for the commands.

## Configure

The config file lives in the platform config dir:
`~/.config/ligus-term/config.toml` on Linux and `~/Library/Application
Support/ligus-term/config.toml` on macOS. Override the path with `--config`
or `LT_CONFIG`. Every setting is optional:

```toml
theme = "auto"   # auto (from COLORFGBG), dark, light, or mono

[keys.global]
quit = ["ctrl-q"]          # replaces the defaults for this action

[keys.composer]
send = ["enter"]
newline = ["alt-enter", "ctrl-j"]
```

`lt --print-config` prints every context, action and default key. `NO_COLOR`
selects the mono theme unless a theme is set explicitly.

### Where `lt` keeps its device

`lt` is its own device, separate from the desktop app and from the `ligus`
CLI:

| | Default | Override |
|---|---|---|
| Data dir (identity, database, keychain) | `~/.ligus-term` | `--data-dir`, `LT_DATA_DIR` |
| Secret store | the OS's (macOS keychain file in the data dir, Secret Service, `systemd-creds`), scope `term` | `LIGUS_SECRET_BACKEND` (`file` for owner-only files) |
| Config | `<config dir>/ligus-term/config.toml` | `--config`, `LT_CONFIG` |

`lt` doesn't read `LIGUS_DATA_DIR`, so a shell set up for the CLI doesn't
make `lt` the same device. It refuses a data dir that overlaps the desktop
app's or the CLI's.

## Develop

See [AGENTS.md](AGENTS.md) for building, testing and running, and
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how it fits together.

## License

MIT. See [LICENSE](LICENSE).
