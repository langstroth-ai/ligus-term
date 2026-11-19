# lt: Ligus in the terminal

`lt` is a terminal client for Ligus multi-agent orchestration. (Ligus is
short for *ligustica*, the Italian honeybee.) Point it at a local setup
(`ligus-local`) or a Kubernetes deployment (`ligus-k8s`, the Helm chart),
and from the terminal you found or join the workspace, admit its `ligus
serve` box and workers, start and watch workflow runs, approve their gates,
see what every agent and host is doing, review and apply proposals, and
talk to agents in chats. A plain line mode makes all of it scriptable for
other agents and CI.

`lt` is a Ligus device in its own right: it has its own keys, signs its
events like the desktop app does, and syncs end to end encrypted through
the workspace's relay.

> **Status: early.** `lt` runs on the Ligus crates (pinned to the ligus
> repo's `release/v0.1.0`). `lt --demo` runs the same UI on an in-memory
> sample. What isn't wired yet is listed in
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#what-is-wired).
>
> **Deliberately not in `lt`:** direct messages, people's online status and
> typing indicators, adding another device of your own, and rotating the
> workspace key. Use the Ligus app for those. `lt` ignores DM invites and
> presence beats it receives from other devices.

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
(no server needed), **join one** by pasting an invite code, or **connect a
deployment** (below). Creating or connecting asks for a passkey and then
shows your 24-word **recovery key** once: write it down.

Joining checks the invite first, the way the desktop does: the founder's
device signature is verified against the keys your other workspaces know
for that person. `lt` shows who the invite is from (or that you share no
workspace with them yet, so compare the safety code), and refuses an
invite whose founder key contradicts what your workspaces know. A second
Enter joins.

A workspace created without a relay stays on this device. To invite others,
start `lt` with `LT_RELAY_URL` set to a relay before creating it: invites
carry the relay, and joining needs one. `lt` refuses to start with an
`LT_RELAY_URL` that isn't an http(s) address.

## Connect to a deployment

A deployment runs `ligus serve` (and, if you add them, `ligus worker`
boxes) for one workspace. Neither founds the workspace and neither prints
an invite: a box only joins a workspace an Admin admitted it to. So the
first person (with `lt`, or the desktop) **founds** the workspace on the
deployment's relay and then **admits** each box.

1. **Get the deployment's relay, room and key.**
   - *Kubernetes* (`deploy/kubernetes/ligus-k8s`, the Helm chart): the
     workspace Secret (`<release>-workspace`) holds `room` and `key`
     (`ligus-k8s secrets workspace --throwaway` makes a trial one):

     ```sh
     kubectl -n NS get secret <release>-workspace -o jsonpath='{.data.room}' | base64 -d
     kubectl -n NS get secret <release>-workspace -o jsonpath='{.data.key}' | base64 -d > key.txt
     ```

     An in-cluster relay (`http://langstroth-relay-sync.<ns>.svc.cluster.local:8443`,
     or `langstroth-relay` for the `trial` scenario) isn't reachable from
     your machine: port-forward it and use the local address.

     ```sh
     kubectl -n <relay-ns> port-forward svc/langstroth-relay-sync 18443:8443   # relay: http://127.0.0.1:18443
     ```

     An external relay (`team-external-relay`) is used as is (`https://…`).
   - *Local* (`deploy/local/ligus-local`): `ligus-local configure --role
     serve --data-dir DIR --relay-url URL [--throwaway-workspace]`, then
     `ligus-local start`. The env file
     (`~/.config/ligus-serve/ligus-serve.env`) names the relay
     (`LIGUS_RELAY_URL`) and the room (`LIGUS_WORKSPACE`); with
     `--throwaway-workspace` the key is in `DIR/throwaway-workspace.key`.
   - A gated relay also needs an access token (`hrt1…`); an open relay
     doesn't.
2. **Found it from `lt`.** First run, or `+` later: choose **Connect a
   deployment**, fill in the relay URL, room, key and (if any) token. `lt`
   refuses when the relay already holds that workspace (then ask one of its
   Admins for an invite). In plain mode:
   `lt --plain connect http://127.0.0.1:18443 k8s-trial-… @key.txt` (`@FILE`
   reads a value from a file, so the key stays off the command line).
3. **Admit the boxes.** People (`m`), then `B`. Paste the box's join
   request (`ligusjoin1:…`, which `ligus serve` or `ligus worker` prints
   when it isn't a member yet, with a fingerprint) and check the
   fingerprint: `lt` seals the admission to that box (`ligusadmit1:…`); on
   the box, `ligus admit redeem <code>` and start it again. Or leave the
   request empty for a bearer code, for a Kubernetes Secret or
   `LIGUS_ADMISSION_FILE`:

   ```sh
   lt --plain admit | cut -f5 > serve.code      # bearer: it carries the workspace key
   kubectl -n NS create secret generic <release>-serve-admission --from-file=admission=serve.code
   # chart values: serve.admission.existingSecret: <release>-serve-admission
   #               worker.instances[].admission.existingSecret: one Secret per worker
   ```

   Admitting takes your passkey (or `--recovery-words-file`). Each box
   joins as its own service member (at most Contributor). Delete the code
   and the Secret once it has joined.
4. **Point agents at hosts.** Agents bound to a worker run there; Hosts
   (`h`) shows each box online or not, busy or idle, and its agents.

### Screens

Press the view's key from anywhere outside a text field:

| Key | View | What's there |
|---|---|---|
| `c` `1` | Chats | Chat list, transcript with streaming agent replies, tool calls, which agents are thinking, the composer with @mention completion, the lockdown bar |
| `f` `2` | Workflows | Workflow definitions and runs, live: each stage's status and output, the gate waiting for approval, who drives the run |
| `p` `3` | Proposals | Code changes, decisions, workflow gates and agent-authored workflows across chats: votes, Approve/Reject, "Apply on ‹host›", and a diff viewer |
| `a` `4` | Agents | Each agent's status (busy, idle, offline), what it says it's doing, its runtime and host; the runtimes; recent hand-offs between agents |
| `h` `5` | Hosts | This device and the workers: online (from heartbeats) or last seen, busy or idle, their agents; the checkout this device applies proposals in |
| `y` `6` | Memory | Agents' live task state (status, progress, open questions) and the workspace memory, read-only |
| `m` `7` | People | Members, roles, founder and service badges, invites, admitting a box. Actions the rank rules refuse are crossed out, with the reason |
| `n` `8` | Notifications | @mentions of you; Enter jumps to the chat |
| `s` `9` | Settings | You, notifications, this device (data dir, secret store), theme and config |
| `w` | Workspaces | The switcher; `+` creates, joins or connects another |
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
| Workflows | `S` start a run (asks for its input) · `A` / `R` approve or reject the run's waiting gate · `X` cancel the run · `enter` on a run opens its chat |
| Proposals | `A` approve · `R` reject · `O` apply on a host · `W` withdraw the apply request |
| Diff viewer | `]` / `[` (or `n` / `N`) next and previous hunk · `}` / `{` next and previous file |
| People | `r` change role · `X` remove · `I` invite · `B` admit a box |
| Agents | `r` change runtime |
| Hosts | `b` set this device's checkout for apply on host |
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

### Workflows

A workflow is a set of stages: agent stages (each runs one agent's turn
with a prompt built from the run's input and earlier outputs) and gates (a
proposal people vote on). Definitions belong to a chat; agents propose new
ones, which appear under Proposals as `workflow` and are added once
approved (with a passkey).

When you start a run, **this device drives it**: it holds the run's
coordinator lease, opens each gate, and sends each stage to its agent. A
stage whose agent runs here streams here; any other stage @mentions its
agent, and the agent's host (a `ligus worker`, or its owner's desktop)
answers it. Keep `lt` running until the run ends (`lt --plain run` does).
Anyone can approve or reject a gate: from `lt`, the desktop or the phone.

### Apply on host

An approved proposal is applied by one host, in its checkout. For this
device to apply one, bind the workspace's checkout: Hosts, then `b`, and
the path to a git checkout (or `lt --plain checkout PATH`). It is kept in
this device's data dir, never synced.

### Plain mode

`lt --plain` (also used automatically when stdin or stdout isn't a
terminal) reads one command per line and writes one record per line: a
keyword, tab-separated fields, text last. New messages, run changes,
proposals and passkey prompts print as they happen, so a screen reader
announces them.

```sh
printf 'chats\nopen 1\nsay @coder fix the threshold\nwait\nproposals\n' | lt --demo --plain
```

**One command, for scripts and other agents:** `lt --plain COMMAND
[ARGS…]` runs one command, prints only its records and exits with a status:
0 on success, 1 when it printed an `error` record (or a run it followed
didn't complete), 2 for an unknown command. `--workspace NAME` picks the
workspace (default: the first). Passkey prompts open the browser, or use
`--recovery-words-file PATH`.

```sh
lt --plain workflows                   # workflow  N  ID  NAME  CHAT  STAGES  startable|refused (why)
lt --plain run 1 "fix the flaky test"  # start workflow 1 and follow it: run, stage records, then run-done
lt --plain runs                        # run  N  ID  STATUS  NAME  DONE/TOTAL  DRIVER  INPUT
lt --plain watch 2                     # follow run 2 until it ends
lt --plain gate 2 approve              # approve run 2's waiting gate (or reject)
lt --plain hosts                       # host  N  ID  LABEL  KIND  online|offline  this device  busy|idle  AGENTS  REPOS
lt --plain agents                      # agent  N  @HANDLE  RUNTIME  HOST  ROLE  busy|idle|offline  DOING
lt --plain proposals                   # proposal  N  ID  STATUS  VOTES/QUORUM  TITLE  APPLY  change|decision|workflow|gate
lt --plain tasks                       # task  CHAT  @AGENT  STATUS  PROGRESS  active|inactive  SUMMARY
```

A `stage` record is `stage RUN STAGE STATUS WHAT GATE OUTPUT-OR-ERROR`. Type
`help` (or `lt --plain help`) for every command.

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
