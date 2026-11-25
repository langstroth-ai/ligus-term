//! `lt help`: everything a person or an agent needs, from the binary.
//!
//! `lt --help` covers the flags, the environment and the exit codes.
//! `lt help TOPIC` explains a task, `lt help COMMAND` one plain-mode
//! command and `lt help --FLAG` one flag. The key table is built from
//! [`crate::keymap::BINDINGS`] and the plain commands from [`COMMANDS`],
//! and tests check that every flag, plain command, record and environment
//! variable `lt` reads is documented here.

use std::fmt::Write as _;

use crate::keymap::{BINDINGS, Context};

/// The exit codes, for `lt --help` and `lt help plain`.
pub const EXIT_CODES: &str = concat!(
    "  0  done (warnings allowed)\n",
    "  1  failed: the error record (or lt doctor's fix) says why; also a run a\n",
    "     one-shot command followed that didn't complete\n",
    "  2  usage: an unknown command, help topic or flag, a bad config file or a\n",
    "     bad value in an LT_ variable\n",
    "  3  a precondition you can fix: a data dir lt can't use, a device it can't\n",
    "     open, no workspace yet, no chat open\n",
    "  4  a person must decide or act (a passkey, a cancelled prompt)\n",
    "  5  not found (a workspace, chat, proposal, run, member, agent)\n",
    "  6  refused by the workspace's rules (a role, a lockdown, an agent's access)\n",
    "  7  offline: the relay can't be reached or timed out\n",
    "  The same table as ligus, the relay and the deploy scripts.",
);

/// One environment variable `lt` reads (itself, or through the Ligus crates).
pub struct EnvVar {
    pub name: &'static str,
    pub default: &'static str,
    pub meaning: &'static str,
}

/// Every environment variable `lt` reads. A test checks the source for
/// reads missing from this list.
pub const ENV: &[EnvVar] = &[
    EnvVar {
        name: "LT_DATA_DIR",
        default: "~/.ligus-term",
        meaning: "this device's data dir (--data-dir wins); never shared with the desktop app or the ligus CLI",
    },
    EnvVar { name: "LT_ASCII", default: "off", meaning: "1/true/on/yes: ASCII only, like --ascii" },
    EnvVar { name: "LT_REDUCE_MOTION", default: "off", meaning: "1/true/on/yes: spinners stand still, like --reduce-motion" },
    EnvVar { name: "PAGER", default: "none: the scrollback", meaning: "where D in a chat shows its whole transcript" },
    EnvVar {
        name: "TMUX",
        default: "-",
        meaning: "set inside tmux: a needs-you notification and a copied code (c) go through tmux's passthrough",
    },
    EnvVar { name: "WAYLAND_DISPLAY", default: "-", meaning: "set: c copies a code with wl-copy too (besides the terminal's OSC 52)" },
    EnvVar { name: "DISPLAY", default: "-", meaning: "set: c copies a code with xclip or xsel too (besides the terminal's OSC 52)" },
    EnvVar { name: "WSL_DISTRO_NAME", default: "-", meaning: "set (WSL): c copies a code with clip.exe too" },
    EnvVar {
        name: "VISUAL",
        default: "$EDITOR, else vi",
        meaning: "the editor ctrl-g in the composer opens the draft in (split on spaces: code --wait)",
    },
    EnvVar { name: "EDITOR", default: "vi", meaning: "the editor ctrl-g uses when VISUAL isn't set" },
    EnvVar { name: "LT_CONFIG", default: "<config dir>/ligus-term/config.toml", meaning: "the config file (--config wins)" },
    EnvVar {
        name: "LT_RELAY_URL",
        default: "none: workspaces stay on this device",
        meaning: "the relay new workspaces sync through, an http(s) URL; needed to invite anyone",
    },
    EnvVar {
        name: "LT_NO_BROWSER",
        default: "off",
        meaning: "1/true/on/yes: a passkey prompt shows its page's address (for a browser on another device that \
                  reaches this one's loopback, e.g. over ssh -L) instead of opening a browser here",
    },
    EnvVar {
        name: "LT_PRESENCE_PORT",
        default: "any free port",
        meaning: "the 127.0.0.1 port a passkey prompt listens on for the browser's answer; fix it to forward it \
                  ahead of time (ssh -L 127.0.0.1:PORT:127.0.0.1:PORT)",
    },
    EnvVar { name: "LT_AGENT_TOOLS", default: "on", meaning: "0/false/off/no turns off agent tools for turns run on this device" },
    EnvVar {
        name: "LT_RUN_COMMAND",
        default: "off",
        meaning: "1/true/on/yes allows the run_command shell tool, where the chat's sandbox and shell rule allow one",
    },
    EnvVar {
        name: "LIGUS_SECRET_BACKEND",
        default: "the OS store",
        meaning: "where lt keeps its secrets: file (owner-only files in the data dir), k8s, secret-service or \
                  systemd-creds; default a keychain file in the data dir on macOS, the Secret Service or \
                  systemd-creds on Linux",
    },
    EnvVar {
        name: "LIGUS_SECRETS_DIR",
        default: "/var/run/secrets/ligus",
        meaning: "the mounted Secret dir the k8s secret backend reads (on Linux, setting it selects that backend)",
    },
    EnvVar {
        name: "LIGUS_MCP_MIN_ROLE",
        default: "admin for a turn that can write, else contributor",
        meaning: "the lowest role (viewer, contributor, admin, owner) whose messages get agent tools in turns run here",
    },
    EnvVar {
        name: "LIGUS_DATA_DIR",
        default: "-",
        meaning: "not used as lt's data dir: read only so lt refuses a data dir that overlaps the ligus CLI's",
    },
    EnvVar {
        name: "ANTHROPIC_API_KEY",
        default: "-",
        meaning: "found by `available`; a local:anthropic agent reads it at each turn (only its presence is checked)",
    },
    EnvVar {
        name: "OPENAI_API_KEY",
        default: "-",
        meaning: "found by `available`; a local:openai agent reads it at each turn (only its presence is checked)",
    },
    EnvVar {
        name: "OPENROUTER_API_KEY",
        default: "-",
        meaning: "found by `available`; a local:openrouter agent reads it at each turn (only its presence is checked)",
    },
    EnvVar { name: "CLAUDE_CODE_OAUTH_TOKEN", default: "-", meaning: "counts Claude Code as signed in (only its presence is checked)" },
    EnvVar {
        name: "COPILOT_GITHUB_TOKEN",
        default: "-",
        meaning: "counts the Copilot CLI as signed in (a classic ghp_ token is flagged: Copilot refuses it)",
    },
    EnvVar { name: "GH_TOKEN", default: "-", meaning: "as COPILOT_GITHUB_TOKEN" },
    EnvVar { name: "GITHUB_TOKEN", default: "-", meaning: "as COPILOT_GITHUB_TOKEN" },
    EnvVar { name: "CLAUDE_CONFIG_DIR", default: "~/.claude", meaning: "where detection looks for Claude Code's sign-in" },
    EnvVar { name: "CODEX_HOME", default: "~/.codex", meaning: "where detection looks for Codex's sign-in" },
    EnvVar { name: "COPILOT_HOME", default: "~/.copilot", meaning: "where detection looks for the Copilot CLI's sign-in" },
    EnvVar { name: "PATH", default: "-", meaning: "where detection finds claude, codex, copilot and agy" },
    EnvVar { name: "HOME", default: "-", meaning: "the default data dir's parent; without it, ./ligus-term-data" },
    EnvVar { name: "NO_COLOR", default: "-", meaning: "any value: the mono theme, unless a theme is named (--theme, theme =)" },
    EnvVar {
        name: "COLORTERM",
        default: "-",
        meaning: "truecolor or 24bit: 24-bit colour; otherwise lt uses the nearest of 256 colours (or 16, see TERM)",
    },
    EnvVar {
        name: "TERM",
        default: "-",
        meaning: "without COLORTERM, a basic terminal (linux, vt100, xterm, screen, *-16color) gets the 16 ANSI \
                  colours, so your terminal's palette applies; anything else 256",
    },
    EnvVar { name: "LT_COLOR_DEPTH", default: "detected", meaning: "truecolor, 256 or 16: overrides the detection" },
    EnvVar {
        name: "COLORFGBG",
        default: "-",
        meaning: "appearance = auto asks the terminal for its background (OSC 11) first; without an answer it reads this (7, 9-15 are light), else dark",
    },
];

/// One plain-mode command.
pub struct Cmd {
    pub name: &'static str,
    pub usage: &'static str,
    pub about: &'static str,
    /// The records it prints (see `lt help plain` for the fields).
    pub prints: &'static str,
    pub example: &'static str,
}

/// Every plain-mode command. A test checks this against the commands plain
/// mode accepts.
pub const COMMANDS: &[Cmd] = &[
    Cmd {
        name: "help",
        usage: "help [COMMAND|TOPIC]",
        about: "List the commands, or explain one command or topic.",
        prints: "help",
        example: "lt --plain help run",
    },
    Cmd {
        name: "name",
        usage: "name TEXT",
        about: "Set your name before the first create or join (it names this device's identity). It lasts \
                for the session, so set it in the same input as create or join; a one-shot command takes \
                --name NAME instead.",
        prints: "ok",
        example: "printf 'name Ada\\ncreate Northwind --out words.txt\\nwait\\n' | lt --plain",
    },
    Cmd {
        name: "create",
        usage: "create NAME [--out FILE]",
        about: "Create a workspace on this device (no server needed; with LT_RELAY_URL it syncs through that \
                relay). Asks for a new passkey in the browser. Prints the 24 recovery words once; with \
                --out FILE they go to a new owner-only file instead. A one-shot create needs --out, and \
                on a new device --name NAME.",
        prints: "created, recovery-key, invite",
        example: "lt --name Ada create Northwind --out words.txt",
    },
    Cmd {
        name: "join",
        usage: "join CODE",
        about: "Join a workspace with an invite code. Check it first with check CODE. Needs no passkey, \
                so it is the way onto a headless device; on a new device add --name NAME.",
        prints: "joined",
        example: "lt --name Ada join hivews1:...",
    },
    Cmd {
        name: "link-request",
        usage: "link-request",
        about: "Ask to be added to your devices: you are already in the workspace on your desktop, and this \
                computer should act as you, not as a second member. Prints this computer's request \
                (liguslink1:..., no secret) and its fingerprint; on your desktop, Settings > Profile > My \
                devices > Add a computer, paste it, compare the fingerprint and confirm with your passkey. \
                An install already in use asks with a new device key. See lt help devices.",
        prints: "link-request, ok",
        example: "lt --plain link-request",
    },
    Cmd {
        name: "link-redeem",
        usage: "link-redeem CODE --approver-fingerprint FP",
        about: "Link this computer with the code your desktop showed (ligusgrant1:..., sealed to this \
                computer). Prints who approved it; links only when FP is the approving desktop's \
                fingerprint (its sheet shows it as This desktop), else exits 4. From then on this computer \
                acts as that person; it no longer creates workspaces or accepts invites.",
        prints: "link-grant, linked",
        example: "lt --plain link-redeem ligusgrant1:... --approver-fingerprint 2C41-88E0-1B9D-04AF-6E73",
    },
    Cmd {
        name: "check",
        usage: "check CODE",
        about: "Say who an invite is from before joining: trusted (a key your workspaces know), unknown \
                (compare the safety code with the sender) or mismatch (refused).",
        prints: "invite-check",
        example: "lt --plain check hivews1:...",
    },
    Cmd {
        name: "connect",
        usage: "connect RELAY ROOM KEY [TOKEN] [--out FILE]",
        about: "Found a deployment's workspace on its relay (ligus serve on Kubernetes or a home server). KEY \
                and TOKEN may be @FILE (owner-only); a one-shot connect takes them only as @FILE and needs \
                --out FILE for the recovery words. Refused when the relay already holds the workspace: ask \
                one of its Admins for an invite. See lt help kubernetes.",
        prints: "connected, recovery-key",
        example: "lt --plain connect http://127.0.0.1:18443 ROOM @key.txt --out words.txt",
    },
    Cmd {
        name: "workspaces",
        usage: "workspaces",
        about: "List this device's workspaces.",
        prints: "workspace",
        example: "lt --plain --json workspaces",
    },
    Cmd {
        name: "use",
        usage: "use N",
        about: "Switch to a workspace (a number from workspaces, or an id). One-shot: use --workspace.",
        prints: "using",
        example: "use 2",
    },
    Cmd {
        name: "chats",
        usage: "chats",
        about: "List the workspace's chats. Flags, space-separated: unread, mentioned, needs-you (a proposal waits on your vote) \
                and locked (a lockdown pauses its agents).",
        prints: "chat",
        example: "lt --plain chats",
    },
    Cmd {
        name: "open",
        usage: "open N",
        about: "Open a chat and print its transcript (and its lockdown, if any).",
        prints: "opened, lockdown, message",
        example: "open 1",
    },
    Cmd { name: "new", usage: "new TITLE", about: "Start a chat and open it.", prints: "opened", example: "new Flaky tests" },
    Cmd {
        name: "say",
        usage: "say TEXT",
        about: "Post to the open chat; @handle mentions an agent. \\n in TEXT is a line break. Replies print \
                as message records when they finish (wait lets them arrive).",
        prints: "sent",
        example: "printf 'open 1\\nsay @coder fix the threshold\\nwait\\n' | lt --plain",
    },
    Cmd {
        name: "stop",
        usage: "stop",
        about: "Stop the agents' turns running on this device in the open chat (an error when none runs here: its host stops it).",
        prints: "stopped",
        example: "stop",
    },
    Cmd {
        name: "asks",
        usage: "asks",
        about: "List the open chat's agent questions and grant cards, and who each waits on. Read-only: you answer them in the full-screen lt (a person's step).",
        prints: "ask",
        example: "asks",
    },
    Cmd { name: "lock", usage: "lock", about: "Lock the open chat down.", prints: "ok", example: "lock" },
    Cmd {
        name: "unlock",
        usage: "unlock",
        about: "Clear the open chat's lockdown (may ask for a passkey).",
        prints: "ok",
        example: "unlock",
    },
    Cmd {
        name: "proposals",
        usage: "proposals",
        about: "List proposals from every chat: code changes, decisions, workflow gates and workflows.",
        prints: "proposal",
        example: "lt --plain --json proposals",
    },
    Cmd { name: "diff", usage: "diff N", about: "Print a proposal's diff.", prints: "file, hunk, line", example: "lt --plain diff 1" },
    Cmd {
        name: "approve",
        usage: "approve N",
        about: "Vote to approve a proposal (some votes ask for a passkey).",
        prints: "voted",
        example: "lt --plain approve 1",
    },
    Cmd { name: "reject", usage: "reject N", about: "Vote to reject a proposal.", prints: "voted", example: "lt --plain reject 1" },
    Cmd {
        name: "apply",
        usage: "apply N [HOST]",
        about: "Ask a host to apply an approved proposal in its checkout. HOST is a number, id or label from \
                hosts; default the first online worker, else this device (which needs a checkout).",
        prints: "apply",
        example: "lt --plain apply 3 build-box",
    },
    Cmd {
        name: "agents",
        usage: "agents",
        about: "List agents: runtime, host, status, what each is doing, who can use it, and for agents you \
                own this device's usage today and this month.",
        prints: "agent, usage",
        example: "lt --plain --json agents",
    },
    Cmd {
        name: "runtimes",
        usage: "runtimes",
        about: "List the workspace's runtimes and whether each is configured.",
        prints: "runtime",
        example: "lt --plain runtimes",
    },
    Cmd {
        name: "hosts",
        usage: "hosts",
        about: "List this device and the workers: online or not, busy or idle, their agents and repos.",
        prints: "host",
        example: "lt --plain hosts",
    },
    Cmd {
        name: "members",
        usage: "members",
        about: "List the workspace's people and boxes, their roles, and whether you may remove each.",
        prints: "member",
        example: "lt --plain members",
    },
    Cmd {
        name: "role",
        usage: "role M ROLE",
        about: "Change a member's role: owner, admin, contributor or viewer (asks for a passkey).",
        prints: "ok",
        example: "lt --plain role 3 contributor",
    },
    Cmd {
        name: "remove",
        usage: "remove M",
        about: "Remove a member from the workspace (asks for a passkey).",
        prints: "ok",
        example: "lt --plain remove 4",
    },
    Cmd {
        name: "access",
        usage: "access N everyone|owner",
        about: "Who may use an agent you own: everyone, or only you (owner, also me). N is a number from \
                agents or a handle.",
        prints: "ok",
        example: "lt --plain access coder everyone",
    },
    Cmd {
        name: "available",
        usage: "available",
        about: "List what this device could run agents on: CLIs on PATH (claude, codex, copilot, agy), a \
                local Ollama's models, and API keys in the environment. Only runs --version; never reads a \
                key. See lt help agents.",
        prints: "available",
        example: "lt --plain --json available",
    },
    Cmd {
        name: "add-agent",
        usage: "add-agent N [NAME]",
        about: "Add runtime N from available as an agent: yours, only you can use it, and it runs on this \
                device. NAME defaults to the suggested one.",
        prints: "ok",
        example: "lt --plain add-agent 2 coder",
    },
    Cmd {
        name: "invite",
        usage: "invite",
        about: "Create an invite code (Contributor). The workspace needs a relay.",
        prints: "invite",
        example: "lt --plain invite",
    },
    Cmd {
        name: "admit",
        usage: "admit [REQUEST] [--out FILE]",
        about: "Admit a box (ligus serve or ligus worker). With its join request (ligusjoin1:...) it first \
                prints the box's fingerprint to compare, then asks for a passkey and prints a sealed code \
                (ligusadmit1:...) for ligus admit redeem on the box. Without one it makes a bearer code, which \
                carries the workspace key, so it needs --out FILE (written owner-only, never printed).",
        prints: "joinrequest, admission",
        example: "lt --plain admit --out serve.code",
    },
    Cmd {
        name: "notifications",
        usage: "notifications",
        about: "List @mentions of you, read or unread, across workspaces.",
        prints: "notification",
        example: "lt --plain notifications",
    },
    Cmd {
        name: "settings",
        usage: "settings",
        about: "Print your name, this device's name and the notification and agent settings.",
        prints: "setting",
        example: "lt --plain --json settings",
    },
    Cmd {
        name: "confirm",
        usage: "confirm ID [file PATH]",
        about: "Answer a passkey prompt (from a presence record): in the browser, or with an owner-only \
                recovery-words file.",
        prints: "presence-done",
        example: "confirm 101 file ~/words.txt",
    },
    Cmd { name: "cancel", usage: "cancel ID", about: "Cancel a passkey prompt.", prints: "presence-done", example: "cancel 101" },
    Cmd {
        name: "workflows",
        usage: "workflows",
        about: "List workflow definitions, their stages and whether you may start each.",
        prints: "workflow",
        example: "lt --plain workflows",
    },
    Cmd { name: "runs", usage: "runs", about: "List workflow runs.", prints: "run", example: "lt --plain --json runs" },
    Cmd {
        name: "run",
        usage: "run W [INPUT]",
        about: "Start workflow W (number, id or name) and follow it until it ends. This device drives the \
                run, so lt stays until it is done. Exit 1 if it doesn't complete.",
        prints: "run, stage, run-done (or error)",
        example: "lt --plain run 1 \"fix the flaky test\"",
    },
    Cmd {
        name: "start",
        usage: "start W [INPUT]",
        about: "Start workflow W and return at once. This device still drives it: keep an lt running \
                (interactive, or watch).",
        prints: "run",
        example: "start 2 this week",
    },
    Cmd {
        name: "watch",
        usage: "watch R",
        about: "Follow run R until it ends. Exit 1 if it doesn't complete.",
        prints: "run, stage, run-done (or error)",
        example: "lt --plain watch 2",
    },
    Cmd {
        name: "gate",
        usage: "gate R approve|reject",
        about: "Decide run R's waiting gate.",
        prints: "gate",
        example: "lt --plain gate 2 approve",
    },
    Cmd { name: "cancel-run", usage: "cancel-run R", about: "Cancel run R.", prints: "run", example: "lt --plain cancel-run 2" },
    Cmd {
        name: "tasks",
        usage: "tasks",
        about: "Agents' live task state: status, progress and summary per chat.",
        prints: "task",
        example: "lt --plain tasks",
    },
    Cmd {
        name: "memory",
        usage: "memory",
        about: "The workspace memory (read-only) and how many memory proposals are open.",
        prints: "memory, memory-proposals",
        example: "lt --plain memory",
    },
    Cmd {
        name: "handoffs",
        usage: "handoffs",
        about: "Recent hand-offs between agents.",
        prints: "handoff",
        example: "lt --plain handoffs",
    },
    Cmd {
        name: "checkout",
        usage: "checkout [PATH|none]",
        about: "Show or set this device's git checkout for the workspace, where proposals applied on this \
                device land. Kept in this device's data dir, never synced.",
        prints: "checkout",
        example: "lt --plain checkout ~/src/northwind-api",
    },
    Cmd {
        name: "setup",
        usage: "setup [FLOW [--FIELD VALUE ...]]",
        about: "The guided setup flows, as ligus setup: with no flow, each flow (agent, tool, folder, box, phone, \
                invite, relay) and what is done here; with one, run it. Flags are the flow's field ids (lt help setup \
                lists them); --dry-run checks and writes nothing. In interactive plain mode a secret takes only \
                @FILE (owner-only); one-shot (lt setup ...) it also takes - for stdin and prints one ligus.checks.v1 \
                document with --json. A step only a person can do ends the run with exit 4 and says where.",
        prints: "setup, check, next, setup-done",
        example: "lt setup agent --harness claude-code --name helper --test-turn",
    },
    Cmd {
        name: "wait",
        usage: "wait [SECS]",
        about: "Interactive only: wait for running actions, then until events go quiet (default up to 30 s).",
        prints: "(the events that arrive)",
        example: "printf 'open 1\\nsay @coder hi\\nwait 20\\n' | lt --plain",
    },
    Cmd {
        name: "quit",
        usage: "quit | exit",
        about: "Interactive only: stop reading commands. EOF does the same after running actions finish.",
        prints: "-",
        example: "quit",
    },
];

/// The fields of every record plain mode prints, by keyword. A record with
/// another field count uses the variant with that many fields; fields past
/// the last name go in `args`. A test checks every record keyword in the
/// plain-mode source is here.
pub const RECORDS: &[(&str, &[&[&str]])] = &[
    ("device", &[&["name", "data_dir", "backend"]]),
    ("first-run", &[&["text"]]),
    ("ready", &[&["text"]]),
    ("help", &[&["text"]]),
    ("ok", &[&["what"]]),
    ("error", &[&["message"], &["chat", "message"]]),
    ("created", &[&["id", "name"]]),
    ("recovery-key", &[&["words", "note"]]),
    ("invite", &[&["code"]]),
    ("joined", &[&["id", "name"]]),
    ("link-request", &[&["fingerprint", "label", "code"]]),
    ("link-grant", &[&["person", "workspace", "approver"]]),
    ("linked", &[&["id", "name", "person"]]),
    ("workspace", &[&["n", "id", "name", "role", "sync"]]),
    ("using", &[&["id", "name"]]),
    ("invite-check", &[&["trust", "workspace", "founder", "safety_code", "relay", "note"]]),
    ("connected", &[&["id", "name", "sync"]]),
    ("joinrequest", &[&["fingerprint", "label"]]),
    ("admission", &[&["kind", "fingerprint", "label", "code"]]),
    ("workflow", &[&["n", "id", "name", "chat", "stages", "start"]]),
    ("run", &[&["n", "id", "status", "name", "done", "driver", "input"]]),
    ("stage", &[&["run", "stage", "status", "what", "gate", "note"]]),
    ("run-done", &[&["id", "status"]]),
    ("gate", &[&["run", "stage", "decision"]]),
    ("task", &[&["chat", "agent", "status", "progress", "state", "summary"]]),
    ("memory", &[&["id", "title", "tags", "author", "body"]]),
    ("memory-proposals", &[&["open"]]),
    ("handoff", &[&["chat", "from", "to", "task"]]),
    ("checkout", &[&["path"]]),
    ("setup", &[&["flow"], &["progress"], &["title"], &["detail"]]),
    ("check", &[&["id"], &["status"], &["title"], &["detail"], &["fix"]]),
    ("next", &[&["command"]]),
    ("setup-done", &[&["flow"], &["exit"], &["state"]]),
    ("chat", &[&["n", "id", "title", "flags"]]),
    ("opened", &[&["id", "title"]]),
    ("lockdown", &[&["reasons", "clear"]]),
    ("message", &[&["workspace", "chat", "author", "text"]]),
    ("sent", &[&["chat", "message"]]),
    ("stopped", &[&["chat", "n"]]),
    ("proposal", &[&["n", "id", "status", "votes", "title", "apply", "kind"]]),
    ("file", &[&["path", "added", "removed"]]),
    ("hunk", &[&["header"]]),
    ("line", &[&["kind", "text"]]),
    ("voted", &[&["id", "status", "votes", "title"]]),
    ("apply", &[&["id", "apply"]]),
    ("agent", &[&["n", "handle", "runtime", "host", "role", "status", "doing", "access", "availability"]]),
    ("usage", &[&["agent", "period", "member", "turns", "uncosted", "cost_usd"]]),
    ("runtime", &[&["id", "kind", "model", "state"]]),
    ("host", &[&["n", "id", "label", "kind", "online", "this_device", "busy", "agents", "repos"]]),
    ("member", &[&["n", "id", "name", "role", "tags", "remove"]]),
    ("available", &[&["n", "detail"], &["n", "id", "kind", "model", "version", "state", "detail", "used", "name"]]),
    ("notification", &[&["workspace", "text"], &["state", "workspace", "text"]]),
    ("setting", &[&["key", "value"]]),
    ("tool", &[&["chat", "tool", "preview"]]),
    ("thinking", &[&["chat", "agent"]]),
    ("proposal-changed", &[&["id", "status", "votes", "title"]]),
    ("ask", &[&["n", "id", "agent", "kind", "status", "question", "options"]]),
    ("asks-changed", &[&["chat"]]),
    ("presence", &[&["id", "kind", "digest", "summary", "answers"]]),
    ("presence-problem", &[&["id", "error"]]),
    ("presence-url", &[&["id", "port", "address", "note"]]),
    ("presence-done", &[&["id", "result", "error"]]),
    ("sync", &[&["workspace", "status"]]),
    ("run-changed", &[&["id", "status", "name", "done"]]),
];

/// Undo plain mode's field escaping (`\n`, `\r`, `\\`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

/// A plain record (keyword, tab, fields) as one JSON object: `record` is
/// the keyword, the fields are named as in [`RECORDS`] (counts as numbers,
/// see [`NUMBER_FIELDS`]; the rest strings), and fields past the names go in
/// `args`.
pub fn record_json(line: &str) -> String {
    let mut parts = line.split('\t');
    let kind = parts.next().unwrap_or("");
    let fields: Vec<String> = parts.map(unescape).collect();
    let variants = RECORDS.iter().find(|(k, _)| *k == kind).map(|(_, v)| *v).unwrap_or(&[]);
    let names: &[&str] = variants
        .iter()
        .find(|v| v.len() == fields.len())
        .or_else(|| variants.iter().filter(|v| v.len() <= fields.len()).max_by_key(|v| v.len()))
        .copied()
        .unwrap_or(&[]);
    let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    let mut out = format!("{{\"record\":{}", q(kind));
    for (name, v) in names.iter().zip(&fields) {
        let _ = write!(out, ",{}:{}", q(name), typed(kind, name, v, &fields).unwrap_or_else(|| q(v)));
    }
    if fields.len() > names.len() {
        let rest: Vec<String> = fields[names.len()..].iter().map(|f| q(f)).collect();
        let _ = write!(out, ",\"args\":[{}]", rest.join(","));
    }
    out.push('}');
    out
}

/// Fields that are numbers in JSON (an empty or `-` value is `null`).
pub const NUMBER_FIELDS: &[&str] = &["n", "added", "removed", "turns", "uncosted", "cost_usd", "progress", "open", "port"];

/// A field's JSON value when it isn't a string: a number for the numeric
/// fields (and a passkey prompt's id), `true`/`false` for a toggle setting.
fn typed(kind: &str, name: &str, v: &str, fields: &[String]) -> Option<String> {
    let number = NUMBER_FIELDS.contains(&name) || (name == "id" && kind.starts_with("presence"));
    if number {
        let v = v.trim_start_matches('+').trim();
        // A diff's removed lines print as -3: the count is 3.
        let v = if name == "removed" { v.trim_start_matches('-') } else { v };
        if v.is_empty() || v == "-" {
            return Some("null".into());
        }
        return v.parse::<f64>().ok().filter(|f| f.is_finite()).map(|_| v.to_string());
    }
    let toggle = fields.first().is_some_and(|k| matches!(k.as_str(), "notify_mentions" | "agents_may_edit"));
    if kind == "setting" && name == "value" && toggle && matches!(v, "true" | "false") {
        return Some(v.to_string());
    }
    None
}

/// `lt help devices`.
const DEVICES: &str = "\
Use lt as one of your devices

Already in a workspace on your desktop? Don't invite yourself: an invite always
makes a new member, and you would show up twice. Add this computer to your
devices instead; it then acts as you (your name, your role, your chats).

1. In lt: first run, choose Add to my devices. In an install you already use,
   press D in Settings or People (they say \"This terminal is its own member\"
   while it is), or type link in the palette (:). lt shows a request (no
   secret) and a fingerprint. c copies the request (ctrl-y once you are pasting
   the desktop's code); o shows it alone on the screen to select it.
2. On your desktop: Settings > Profile > My devices > Add a computer. Paste the
   request, compare all five groups of the fingerprint, confirm with your passkey.
3. Paste the code the desktop shows into lt (within 30 minutes). lt shows who
   approved it and the desktop's fingerprint (the desktop shows it as This
   desktop): check it is yours and confirm.

A linked lt acts as you without your account key: a vote asks for your passkey
in the browser, and account changes (devices, phones, passkeys, keys) stay on
the desktop. It no longer creates workspaces or accepts invites. To remove it,
revoke it in My devices on the desktop.

Already invited yourself? An install already in a workspace as its own member
asks with a new device key; the desktop then offers to remove that old member
(their messages keep their name). On the desktop, People also points at a
member that looks like you on another computer, and its row menu opens Add a
computer for it.
";

/// The topics `lt help TOPIC` knows, with a line each.
pub const TOPICS: &[(&str, &str)] = &[
    ("quickstart", "install lt, then create your first workspace or join one"),
    ("keys", "every key in the full-screen app, and how to rebind them"),
    ("plain", "scripting with --plain: one-shot commands, records, --json, exit codes"),
    ("agents", "adding agents from the runtimes this device can run, and who can use them"),
    ("devices", "use lt as one of your devices (Add to my devices), not as a second member"),
    ("kubernetes", "connecting to a Kubernetes (or home server) deployment and admitting its boxes"),
    ("troubleshooting", "refusals, keychain prompts, passkeys without a browser, and other fixes"),
    ("env", "every environment variable lt reads"),
    ("completions", "tab completion for bash, zsh and fish, and the man page"),
    ("doctor", "lt doctor: what lt sees of this machine and device, as a ligus.checks.v1 report"),
    ("setup", "the guided setup flows: an agent, a tool, a folder, a box, a phone, an invite, a relay"),
    ("themes", "the palettes, light and dark, and your own theme files (lt themes list, lt themes check)"),
];

/// What `lt --help` prints after the flags.
pub fn after_long_help() -> String {
    let mut s = String::from("Help topics (lt help TOPIC):\n");
    for (t, about) in TOPICS {
        let _ = writeln!(s, "  {t:<16} {about}");
    }
    s.push_str("\nPlain-mode commands (lt help COMMAND for each; lt --plain COMMAND runs one):\n");
    let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    for line in wrap(&names.join(" "), 76) {
        let _ = writeln!(s, "  {line}");
    }
    s.push_str("\nEnvironment:\n");
    s.push_str(&env_text());
    s.push_str("\nExit codes:\n");
    s.push_str(EXIT_CODES);
    s.push_str(
        "\n\nExamples:\n  lt --demo                          explore with sample workspaces (keeps nothing)\n  \
         lt                                 first run: your name, then create, join or connect\n  \
         lt --plain --json agents           one command, JSON Lines out\n  \
         lt --plain run 1 \"fix the tests\"   start workflow 1 and follow it to the end\n  \
         lt help quickstart                 the first steps\n  \
         lt completions zsh                 tab completion (bash, zsh, fish); lt man prints the man page\n\
         \nLigus is short for ligustica, the Italian honeybee.",
    );
    s
}

fn env_text() -> String {
    let mut s = String::new();
    for v in ENV {
        match v.default {
            "-" => _ = writeln!(s, "  {}", v.name),
            d => _ = writeln!(s, "  {}  (default: {d})", v.name),
        }
        for line in wrap(v.meaning, 72) {
            let _ = writeln!(s, "      {line}");
        }
    }
    s
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![];
    let mut cur = String::new();
    for w in text.split_whitespace() {
        if !cur.is_empty() && cur.len() + 1 + w.len() > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// One command's help.
pub fn command_text(c: &Cmd) -> String {
    let mut s = format!("{}\n\n", c.usage);
    for line in wrap(c.about, 76) {
        let _ = writeln!(s, "  {line}");
    }
    let _ = writeln!(s, "\n  Prints: {}", c.prints);
    for kind in c.prints.split([',', ' ']).filter(|k| !k.is_empty()) {
        if let Some((_, v)) = RECORDS.iter().find(|(k, _)| *k == kind) {
            let fields: Vec<String> = v.iter().map(|f| f.join(" ")).collect();
            let _ = writeln!(s, "    {kind}: {}", fields.join(" | "));
        }
    }
    let _ = writeln!(s, "\n  Example: {}", c.example);
    s
}

/// The key table, built from the keymap's own binding table.
pub fn keys_text() -> String {
    let mut s = String::from(
        "Keys in the full-screen app (the defaults; `?` shows yours as configured,\n\
         what has focus first; / in it searches).\n\
         Press a view's key from anywhere outside a text field. While a text field\n\
         has focus, letters type: only ctrl and alt chords and function keys reach\n\
         the Anywhere keys.\n",
    );
    for ctx in Context::ALL {
        let _ = writeln!(s, "\n{} [keys.{}]", ctx.title(), ctx.name());
        for b in BINDINGS.iter().filter(|b| b.context == ctx) {
            let _ = writeln!(s, "  {:<24} {:<20} {}", b.keys.join(" / "), b.name, b.help);
        }
    }
    s.push_str(
        "\nPasskey prompt (fixed keys)\n  enter / o                open the browser\n  \
         f                        use a recovery-words file (confirmations only)\n  \
         esc / c                  cancel\n\
         \nRebind in the config file (lt --print-config prints them all), per context,\n\
         by action name; an entry replaces that action's keys in that context:\n\n  \
         [keys.global]\n  quit = [\"ctrl-q\"]\n  [keys.composer]\n  newline = [\"alt-enter\", \"ctrl-j\"]\n\n\
         Keys are written like ctrl-c, alt-enter, shift-tab, f1, enter, esc, tab, space,\n\
         up, down, left, right, home, end, pageup, pagedown, or one character (G is shift-g).\n\
         A key bound twice in one context is refused at start-up (exit 2).\n",
    );
    s
}

fn plain_text() -> String {
    let mut s = String::from(
        "\
Scripting lt with plain mode

lt --plain reads one command per line on stdin and writes one record per line:
a keyword, then tab-separated fields, text last. It is also used when stdin or
stdout isn't a terminal. Events (new messages, run changes, proposals, passkey
prompts) print as they arrive. Text from the network has control characters,
escape sequences and bidi controls replaced by U+FFFD, and a line break inside a
field is written \\n (a backslash is \\\\), so it can't forge a record.

  printf 'chats\\nopen 1\\nsay @coder fix it\\nwait\\nproposals\\n' | lt --plain

One-shot: lt COMMAND [ARGS...] (or lt --plain COMMAND ...) runs one command,
prints only its records (and any passkey prompt it raises) and exits. On a new
device, --name NAME names its identity for create, join or connect.
--workspace picks the workspace. Passkey confirmations open the browser, or use
--recovery-words-file PATH. A new passkey (create, connect) needs a browser:
here, or with LT_NO_BROWSER=1 one on another device (a presence-url record;
lt help troubleshooting).

  lt --plain runs
  lt --plain --workspace Northwind run 1 \"fix the flaky test\"

Secrets never ride argv or stdout in one-shot mode: connect takes the key and
token only as @FILE (owner-only), and create, connect and a bearer admit write
their secret to a new owner-only --out FILE.

JSON: --json (or a trailing --json on a one-shot listing command) writes each
record as one JSON object per line. \"record\" is the keyword; the fields are
named as below and unescaped (real line breaks); fields past the names go in
\"args\". Counts are JSON numbers (n, added, removed, turns, uncosted,
cost_usd, progress, open, and a presence record's id), null when missing;
a true/false setting is a boolean; everything else is a string.

  lt --plain --json agents
  lt --plain hosts --json
  {\"record\":\"workspace\",\"n\":1,\"id\":\"...\",\"name\":\"Northwind\",\"role\":\"owner\",\"sync\":\"...\"}

Records (keyword: fields; | separates variants by field count). An ok record
names what was done, then that command's arguments (args in JSON).
",
    );
    for (k, v) in RECORDS {
        let fields: Vec<String> = v.iter().map(|f| f.join(" ")).collect();
        let _ = writeln!(s, "  {k:<17} {}", fields.join(" | "));
    }
    s.push_str("\nCommands (lt help COMMAND for each):\n");
    for c in COMMANDS {
        let _ = writeln!(s, "  {}", c.usage);
    }
    s.push_str("\nExit codes:\n");
    s.push_str(EXIT_CODES);
    s.push('\n');
    s
}

const DOCTOR: &str = "\
lt doctor [--json]

Checks what lt sees, without opening the device: no database, no secret
store (so no keychain prompt) and no network. It reads the data dir's
workspace list, the config file and the environment, and looks for agent
runtimes on PATH the way the Agents view does.

  data_dir.usable        the data dir is lt's own and owner-only
  config.valid           the config file parses and its keys don't clash
  themes.valid           theme files load and the theme named exists (when there are any)
  secret_store.kind      which secret store holds the keys
  relay.url              LT_RELAY_URL, when set, is an http(s) URL
  workspaces.list        how many workspaces, and which sync through a relay
  relay.reachable        skipped: check each relay where it runs (see next)
  runtimes.found         the agent runtimes this device has
  terminal.capabilities  TERM and the colour depth lt picked

Text: one line per check (ok, warn, fail or skip), an indented fix: line,
then a summary and what to run next. --json prints one document in the
shared shape every Ligus doctor uses:

  {\"schema\":\"ligus.checks.v1\",\"tool\":\"lt\",\"version\":\"...\",\"command\":\"doctor\",
   \"ok\":true,\"exitCode\":0,\"checks\":[{\"id\",\"title\",\"status\",\"detail\",\"fix\"}],
   \"next\":[\"langstroth-relay check --url https://relay.example.com\",\"ligus doctor\"]}

Exit 0 when nothing failed (warnings allowed), 1 when a check failed.
";

const THEMES: &str = "\
Themes: the palettes, light and dark, and your own

lt draws with the same six palettes as the desktop and the phone, each for a
light and a dark terminal, on the terminal's own background:

  pollen    honey and gold (the default)
  studio    neutral graphite
  harbor    ocean blue
  meadow    botanical green
  slate     cool blue-grey
  obsidian  monochrome silver (re-lit for a light terminal)
  mono      no colour: bold and reverse only

In config.toml (lt --print-config shows it), or --theme and --appearance:

  theme = \"harbor\"       a palette, a theme file's name, or mono
  appearance = \"auto\"    auto (the terminal's background), light or dark

The old values still work: theme = auto, dark or light is Pollen with that
appearance. NO_COLOR gives mono unless a theme is named. Settings has a Theme
row and an Appearance row (enter cycles them), and the palette (: or ctrl-k)
lists every theme: moving onto one shows it, enter keeps it for the session,
esc goes back. A terminal without 24-bit colour (no COLORTERM=truecolor) gets
the nearest of its 256 or 16; LT_COLOR_DEPTH=truecolor|256|16 overrides that.

Your own theme
  A file in the themes directory next to config.toml, themes/NAME.toml (or
  NAME.json), in the shared Ligus theme format: the same file the desktop
  imports (Settings, Appearance, Import a theme) and exports. NAME is what
  theme = names: lowercase letters, digits, - and _.

    name = \"Heather\"          # shown in Settings (default: the file name)
    extends = \"slate\"         # the palette that gives every colour left out
    [light]
    ink = \"#2b2118\"
    accentCool = \"oklch(0.45 0.12 320)\"
    [dark]
    accentCool = \"#d7a6e8\"

  Colours: #rgb, #rrggbb, #rrggbbaa; rgb(r, g, b), rgba(r, g, b, a) or
  rgb(r g b / a); oklch(L C H) or oklch(L C H / a). Nothing else: no names,
  no CSS. A translucent colour is painted over the canvas.

  The tokens lt draws with:
    canvas       the background contrast is checked against (lt never paints it)
    ink          text
    inkMuted     secondary text (default: ink at 72% over the canvas)
    accentCool   the accent: headings, focus, the selected view
    accentWarm   needs you and @mentions
    border       pane borders (default: the least ink that keeps 3:1)
    selection    the selected row's background (default: an accent tint)
    agent        agents' names
    ok info warn danger neutral   done, working, queued, failed, paused
    diffAddInk diffDelInk diffComment   added and removed lines, comments
  The desktop's own tokens (panel, mist, line, onAccent, sidebarTop,
  sidebarBottom, sidebarInk, sidebarInkMuted, diffAddBg, diffDelBg) are
  accepted and left to it. The schema:
  https://docs.langstroth.ai/schemas/ligus-theme.v1.json

  A bad colour, a value that isn't a string or a file over 64 KiB is an error
  with its line, and the theme doesn't load. An unknown key, or text under
  4.5:1 (a border under 3:1) on the canvas, is a warning and it still loads.
  An accent or status colour too faint for text (a desktop fill colour) is
  shown as the nearest readable shade of its hue, with a warning.

lt themes list [--json]
  The palettes and every theme file, with what each has.

lt themes check FILE [--json]
  Read one file (a path, or a name in the themes directory) and print every
  error and warning; --json is a ligus.checks.v1 report. Exit 0 when it loads,
  1 when it doesn't, 5 when there is no such file.
";

const COMPLETIONS: &str = "\
Tab completion and the man page

lt prints both itself, without opening a device:

  lt completions bash > ~/.local/share/bash-completion/completions/lt
  lt completions zsh  > \"${fpath[1]}/_lt\"     (a directory on your $fpath)
  lt completions fish > ~/.config/fish/completions/lt.fish
  lt man > ~/.local/share/man/man1/lt.1           (then: man lt)

Homebrew installs all of them with lt. The scripts complete flags, commands,
help topics, themes, and workspace names after --workspace and use. Workspace
names come from the data dir's workspace list, read as a plain file: tab
completion never opens the database or the secret store, so it can't cause a
keychain prompt. Chats, proposals and agents aren't completed for that reason.
";

const QUICKSTART: &str = "\
Quick start

Install (one of):
  brew install langstroth-ai/ligus/lt
  cargo install --locked --git https://github.com/langstroth-ai/ligus-term --tag v0.1.1
  or a binary from https://github.com/langstroth-ai/ligus-term/releases
  (lt-<target> with lt-<target>.sha256: check it with shasum -a 256 -c)
  lt --version

Look around first (in memory, keeps nothing):
  lt --demo

Your first workspace:
  1. Decide where it syncs. Without a relay a workspace stays on this device and
     nobody can join it. To invite people, start lt with a relay:
       LT_RELAY_URL=https://relay.example.com lt
  2. lt asks your name, then: create a workspace, join one, or connect a deployment.
  3. Create asks for a new passkey in your browser, then shows 24 recovery words
     once. Write them down: they answer passkey prompts when no browser is at hand.
  4. Invite people: People (m), then I; or lt --plain invite.

Joining a workspace:
  Paste the invite code someone sent you. lt first shows who it is from: a key
  your workspaces already know, or someone new (compare the safety code with the
  sender), and refuses an invite whose founder key contradicts what your
  workspaces know. A second enter joins.
    lt --plain check CODE
    printf 'name Ada\\njoin CODE\\nwait\\n' | lt --plain

A deployment (ligus serve on Kubernetes or a home server): lt help kubernetes.

lt is its own device, with its own keys and its own data dir (~/.ligus-term,
or --data-dir / LT_DATA_DIR), separate from the desktop app and the ligus CLI.

Next: lt help keys, lt help agents, lt help plain.
";

const AGENTS: &str = "\
Adding agents

lt finds what this device could run agents on, the way the desktop app does:
  - CLIs on PATH: Claude Code (claude), Codex (codex), the GitHub Copilot CLI
    (copilot), the Google Antigravity CLI (agy). Gemini CLI is listed only to say
    it can't be added.
  - A local Ollama at 127.0.0.1:11434: one row per installed model.
  - Hosted APIs whose key is set: ANTHROPIC_API_KEY, OPENAI_API_KEY,
    OPENROUTER_API_KEY.
Detection only runs --version, never sends a prompt, never opens a keychain and
never reads a key's value. Each row says whether it is ready, or what to run
(codex login, copilot login, ollama pull).

In the app: Agents (a), then N: pick a runtime in the popup, enter, name the
agent, enter. In plain mode:
  lt --plain available          # the list
  lt --plain add-agent 2 coder  # row 2 as @coder

A new agent is yours, only you can use it, and it runs on this device. Its
runtime is device-local (local:codex, local:ollama/qwen3:8b): a CLI uses its own
sign-in; a hosted API reads its key from this device's environment at each turn.
Then:
  - E (or lt --plain access N everyone|owner) opens it to everyone or back to
    only you. In a workspace on rules v2 opening it asks for your passkey.
  - r changes an agent's runtime.
  - @handle in a chat talks to it.
  - An agent bound to a worker runs there; Hosts (h) shows where each runs.

Tools in turns run here: a runtime with the tool loop (a local endpoint such as
Ollama) gets file tools (private endpoints only) and the Ligus tools. Hosted APIs
get no tool loop. LT_AGENT_TOOLS=0 turns tools off on this device;
LT_RUN_COMMAND=1 allows the run_command shell where the chat allows one.
LIGUS_MCP_MIN_ROLE sets the lowest role whose messages get tools here.
Edits land in a per-turn worktree of the bound checkout (lt --plain checkout
PATH) and come back as a proposal.

Commands:
";

const KUBERNETES: &str = "\
Connecting to a Kubernetes deployment

A deployment runs ligus serve (and optionally ligus worker boxes) for one
workspace. A box never founds a workspace: the first person founds it on the
deployment's relay with lt (or the desktop), then admits each box.

1. Get the relay, room and key. The chart's workspace Secret
   (<release>-workspace) holds them; ligus-k8s secrets workspace --throwaway
   makes a trial one.
     kubectl -n NS get secret <release>-workspace -o jsonpath='{.data.room}' | base64 -d
     (umask 077; kubectl -n NS get secret <release>-workspace -o jsonpath='{.data.key}' | base64 -d > key.txt)
   An in-cluster relay isn't reachable from your machine; port-forward it:
     kubectl -n RELAY_NS port-forward svc/langstroth-relay-sync 18443:8443
     # relay: http://127.0.0.1:18443
   An external relay is used as is. A gated relay also needs an access token
   (hrt1...); an open relay doesn't.
   On a home server (ligus-local): the env file ~/.config/ligus-serve/ligus-serve.env
   names the relay (LIGUS_RELAY_URL) and room (LIGUS_WORKSPACE); with
   --throwaway-workspace the key is in DIR/throwaway-workspace.key.

2. Found the workspace from lt. In the app: first run, or + later, then
   \"Connect a deployment\". In plain mode:
     lt --plain connect http://127.0.0.1:18443 ROOM @key.txt [@token.txt] --out words.txt
   The key and token must be owner-only @FILEs; the recovery words go to the new
   owner-only --out file. It asks for a new passkey in the browser. lt refuses if
   the relay already holds that workspace: ask one of its Admins for an invite.

3. Admit the boxes. In the app: People (m), then B. Paste the join request the
   box printed (ligusjoin1:...) and compare the fingerprint with the box; lt
   seals a code to that box (ligusadmit1:...). On the box: ligus admit redeem
   CODE, then start it again. Or make a bearer code for a Kubernetes Secret:
     lt --plain admit --out serve.code
     kubectl -n NS create secret generic <release>-serve-admission --from-file=admission=serve.code
     # chart values: serve.admission.existingSecret: <release>-serve-admission
     #               worker.instances[].admission.existingSecret (one per worker)
   A bearer code carries the workspace key: it is only written to the new
   owner-only file, never printed. Admitting asks for your passkey (or
   --recovery-words-file). Each box joins as its own service member (at most
   Contributor). Delete the code and the Secret once it has joined.

4. Point agents at hosts: agents bound to a worker run there. lt --plain hosts
   shows each box online or not, busy or idle.
";

const TROUBLESHOOTING: &str = "\
Troubleshooting

\"overlaps the desktop app's data\" / \"overlaps the ligus CLI's data\" (exit 3)
  lt is its own device. Give it its own dir: --data-dir PATH or LT_DATA_DIR.
  lt never uses LIGUS_DATA_DIR as its data dir; it only refuses to overlap it.

A keychain password prompt on macOS, or no keychain at all (a container, CI)
  Keep lt's secrets in owner-only files in its data dir:
    LIGUS_SECRET_BACKEND=file lt
  On Linux without a Secret Service session or systemd-creds lt falls back to
  those files and warns.

\"LT_RELAY_URL ... isn't a relay address\"
  Give an http(s) URL, e.g. LT_RELAY_URL=https://relay.example.com.

\"this workspace has no relay\" (invite, admit)
  It was created without LT_RELAY_URL, so nobody can reach it. Create a new
  workspace with LT_RELAY_URL set.

\"the relay already holds this workspace\" (connect)
  Someone founded it already: ask one of its Admins for an invite and join.

\"tell us your name first\" (a one-shot create or join on a new device)
  Give your name with --name:
    lt --name Ada create Northwind --out words.txt
    lt --name Ada join CODE

Passkeys with no browser (SSH, a headless box)
  Confirmations take your 24 recovery words from an owner-only file: press f
  at the prompt, use confirm ID file PATH, or start with --recovery-words-file.
  \"others can read ...\": chmod 600 the file.

  Creating a passkey (create, connect) needs a browser somewhere, because the
  passkey lives in a browser's authenticator (Touch ID, a phone, a security
  key) and the passkey page hands its answer back to lt on 127.0.0.1. With no
  browser on this device, use one on another device that reaches this one's
  loopback over SSH:
    # on the device with the browser:
    ssh -L 127.0.0.1:8765:127.0.0.1:8765 THIS-HOST   (never with -g)
    # on THIS-HOST, in that session:
    LT_NO_BROWSER=1 LT_PRESENCE_PORT=8765 lt --name Ada create Northwind --out words.txt
  lt shows the page's address on your terminal (below the prompt in the app;
  in plain mode on the controlling terminal, never on stdout, so a log or a
  pipe never holds it) instead of opening a browser; open it on the other
  device. The address holds a one-time secret for that prompt: anyone who
  has it and can reach the port could answer the prompt, so don't paste it
  anywhere else. Without a terminal (lt's output captured, no tty) it isn't
  shown at all. Printing the address alone isn't enough: without the port forward the
  other browser's answer goes to its own 127.0.0.1, not to lt.

  To put a headless device in a workspace without any passkey, join instead:
  create the workspace where a browser is (the desktop app, or lt on your
  laptop), invite this device, then lt --name NAME join CODE here.

\"unknown command\" (exit 2)
  lt help lists the commands; lt help COMMAND explains one. A command runs
  with or without --plain: lt agents and lt --plain agents are the same.

\"... is bound to both ...\" (exit 2)
  Two actions share a key in one context of your config file. lt --print-config
  shows the defaults; lt help keys explains the format.

A run stops moving
  The device that started a run drives it. Keep that lt open (lt --plain run
  and watch stay until it ends).

An agent never answers
  Its host must be online (lt --plain hosts). A personal agent answers only
  its owner until opened with access N everyone. The chat's own runtime runs on
  this device's credentials, so in a shared chat lt answers it only for you.

\"available\" finds nothing, or a row is not ready
  Install the CLI and sign in (codex login, copilot login), start Ollama and
  ollama pull a model, or export an API key, then list again. Claude Code's
  macOS sign-in lives in the keychain, which detection never opens: it shows as
  unchecked and still counts as ready. A classic GitHub token (ghp_) is flagged
  because the Copilot CLI refuses it.

Apply on this device does nothing
  Bind the checkout first: Hosts (h), then b, or lt --plain checkout PATH.

brew install says lt conflicts with localtunnel
  Homebrew's localtunnel also installs an lt binary; uninstall one of them.

Can't select text with the mouse
  lt takes clicks and the wheel: hold shift while dragging (option in some
  terminals), or put mouse = false in the config file.

Colours look wrong
  --theme PALETTE and --appearance light|dark, or theme and appearance in the
  config (lt help themes); NO_COLOR gives mono. With appearance = auto, lt asks
  the terminal for its background at start (OSC 11, 100 ms at most), then
  reads COLORFGBG. Settings (or the palette: type theme) tries another for the
  session, shown as you move. --ascii (or
  LT_ASCII=1) drops box drawing and symbols for limited fonts.
  Settings shows the colour depth lt picked: 24-bit with COLORTERM=truecolor,
  else 256, or 16 on a basic TERM. LT_COLOR_DEPTH=truecolor|256|16 overrides it
  (tmux without truecolor passthrough wants 256).

Still stuck: lt --version, then open an issue at
https://github.com/langstroth-ai/ligus-term/issues (never paste recovery
words, keys or invite codes).
";

/// The text for `lt help TOPIC|COMMAND|--FLAG`, or `None` if unknown. Flags
/// come from `flags`: (long name, value name, help).
pub fn topic(name: &str, flags: &[(String, Option<String>, String)]) -> Option<String> {
    let name = name.trim();
    let mut s = match name {
        "quickstart" => QUICKSTART.to_string(),
        "keys" | "keymap" => keys_text(),
        "plain" | "json" | "scripting" => plain_text(),
        "agents" => {
            let mut s = AGENTS.to_string();
            for n in ["available", "add-agent", "access", "agents", "runtimes"] {
                if let Some(c) = COMMANDS.iter().find(|c| c.name == n) {
                    s.push('\n');
                    s.push_str(&command_text(c));
                }
            }
            s
        }
        "devices" | "link" | "add-to-my-devices" => {
            let mut s = DEVICES.to_string();
            for n in ["link-request", "link-redeem"] {
                if let Some(c) = COMMANDS.iter().find(|c| c.name == n) {
                    s.push('\n');
                    s.push_str(&command_text(c));
                }
            }
            s
        }
        "kubernetes" | "k8s" | "deploy" => KUBERNETES.to_string(),
        "troubleshooting" | "trouble" => TROUBLESHOOTING.to_string(),
        "env" | "environment" => format!("Environment variables lt reads\n\n{}", env_text()),
        "completions" | "man" => COMPLETIONS.to_string(),
        "doctor" | "checks" => DOCTOR.to_string(),
        "setup" | "set-up" => {
            let mut s = crate::setup::cli::all_help();
            if let Some(c) = COMMANDS.iter().find(|c| c.name == "setup") {
                s.push('\n');
                s.push_str(&command_text(c));
            }
            s
        }
        "themes" | "theme" | "colours" | "colors" | "appearance" => THEMES.to_string(),
        "exit" | "exit-codes" => format!("Exit codes\n{EXIT_CODES}\n"),
        _ => String::new(),
    };
    if s.is_empty() {
        if let Some(c) = COMMANDS.iter().find(|c| c.name == name || (name == "exit" && c.name == "quit")) {
            s = command_text(c);
        } else if let Some((long, value, help)) = flags.iter().find(|(l, ..)| name.trim_start_matches('-') == l) {
            s = format!("--{long}{}\n\n", value.as_ref().map(|v| format!(" <{v}>")).unwrap_or_default());
            for line in wrap(help, 76) {
                let _ = writeln!(s, "  {line}");
            }
        } else {
            return None;
        }
    }
    Some(s)
}

/// The list printed for an unknown topic.
pub fn unknown(name: &str) -> String {
    let topics: Vec<&str> = TOPICS.iter().map(|(t, _)| *t).collect();
    format!("no help for \u{201c}{name}\u{201d}. Topics: {}. Commands: lt help plain. Flags: lt --help.", topics.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_names_fields_and_unescapes() {
        assert_eq!(
            record_json("workspace\t1\tws-1\tNorthwind\towner\tsynced"),
            r#"{"record":"workspace","n":1,"id":"ws-1","name":"Northwind","role":"owner","sync":"synced"}"#
        );
        assert_eq!(record_json("error\tc-1\tline\\none"), r#"{"record":"error","chat":"c-1","message":"line\none"}"#);
        assert_eq!(record_json("ok\taccess\t@coder\teveryone"), r#"{"record":"ok","what":"access","args":["@coder","everyone"]}"#);
        assert_eq!(record_json("ok\tlock"), r#"{"record":"ok","what":"lock"}"#);
        assert_eq!(record_json("line\t+\ta \\\\n b"), r#"{"record":"line","kind":"+","text":"a \\n b"}"#);
        // Counts are numbers, a missing one is null, a setting is a bool.
        assert_eq!(record_json("file\tsrc/a.rs\t+9\t-3"), r#"{"record":"file","path":"src/a.rs","added":9,"removed":3}"#);
        assert_eq!(
            record_json("usage\t@coder\tmonth\tMara\t41\t0\t0.1800"),
            r#"{"record":"usage","agent":"@coder","period":"month","member":"Mara","turns":41,"uncosted":0,"cost_usd":0.1800}"#
        );
        assert_eq!(
            record_json("task\tc-1\t@coder\tworking\t\tactive\tx"),
            r#"{"record":"task","chat":"c-1","agent":"@coder","status":"working","progress":null,"state":"active","summary":"x"}"#
        );
        assert_eq!(record_json("setting\tnotify_mentions\ttrue"), r#"{"record":"setting","key":"notify_mentions","value":true}"#);
        assert_eq!(record_json("setting\tname\ttrue"), r#"{"record":"setting","key":"name","value":"true"}"#);
        assert_eq!(record_json("presence-done\t101\tok\t"), r#"{"record":"presence-done","id":101,"result":"ok","error":""}"#);
    }

    /// The variables detection reads (by presence, or as a config dir) are
    /// all documented.
    #[test]
    fn detection_vars_are_documented() {
        use crate::backend::real::detect::{DIR_VARS, PRESENCE_VARS};
        for v in PRESENCE_VARS.iter().chain(DIR_VARS) {
            assert!(ENV.iter().any(|e| e.name == *v), "{v} isn't in help::ENV");
        }
    }

    #[test]
    fn every_topic_renders() {
        for (t, _) in TOPICS {
            assert!(topic(t, &[]).is_some_and(|s| s.len() > 100), "{t}");
        }
        assert!(topic("nonsense", &[]).is_none());
    }
}
