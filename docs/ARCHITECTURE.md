# lt architecture

`lt` is a terminal client for Ligus multi-agent orchestration (Ligus is
short for *ligustica*), against a local setup or a Kubernetes deployment:
founding or joining the workspace, admitting its boxes, driving and
watching workflow runs, deciding gates, watching agents and hosts,
reviewing and applying proposals, and chatting with agents. It is its own
device: its own keys and data dir, signing events like the desktop app.
This document covers how the app is put together and how it is wired to
the Ligus Rust crates.

**Deliberately out of scope:** direct messages, people's online status and
typing indicators, linking another device of your own, and rotating the
workspace key. `lt` never publishes or reads people's presence beats, and
refuses a pasted DM invite with a pointer to the Ligus app; DMs and beats
from other devices never reach its UI (`tests/orchestration.rs` publishes
both and checks).

## Layers

```
            ┌───────────── terminal ─────────────┐   ┌──── stdin/stdout ────┐
            │ crossterm EventStream (keys, paste)│   │  plain.rs (--plain)  │
            └───────────────┬────────────────────┘   └──────────┬───────────┘
                            │ Msg::Key / Msg::Paste             │ commands
                            ▼                                   │
   Msg::Tick ──► app::update(&mut State, Msg) -> Vec<Effect>    │
                            │            ▲                      │
                 ui::draw(&State)        │ Msg::<Result>        │
                            │            │                      │
                            ▼            │                      ▼
              app::runtime::run_effect(Arc<dyn Backend>, Effect)  ──►  Backend
                                         ▲                                │
                                         └──── Msg::Event(Event) ◄── events()
```

- **`backend`**: the `Backend` trait, the view models (`model.rs`), the
  in-memory `fake`, and `presence` (the browser hand-off and the
  recovery-words file).
- **`app::state`**: everything on screen, as plain data: the phase (loading,
  onboarding, recovery key, main), the active workspace and view, focus,
  lists and selections, the composer, overlays, the passkey sheets, toasts.
- **`app::update`**: the reducer. It takes a `Msg` (a key, a paste, a tick,
  a backend event, or an effect's result), changes `State`, and returns the
  `Effect`s (backend calls) to make. It never awaits or draws, so it is
  unit-tested with plain values (`tests/reducer.rs`).
- **`app::runtime`**: the loop. Terminal input, the backend's event stream
  and a 250 ms tick all become `Msg`s. Each `Effect` runs as its own task,
  because some wait on a person (a passkey prompt) and must not block
  others. The screen is redrawn once per batch of messages.
- **`ui`**: drawing only. Each view is a function from `&State` to a frame
  (`chat`, `proposals`, `views` for agents/hosts/people/notifications/settings,
  `onboarding`, `overlays`). Snapshot-tested with ratatui's `TestBackend`
  (`tests/screens.rs`).
- **`keymap`**: actions, contexts, defaults and overrides (one table,
  `BINDINGS`, feeds resolution, the help overlay, the command palette and
  `--print-config`).
- **`app::palette`**: the command palette's entries (the focused view's
  actions, everything global, the views and the loaded entities) and its
  fuzzy match; pure, like the reducer.
- **`theme_tokens`**: the shared Ligus colour tokens, generated in the
  ligus repo (`assets/branding/generated/lt/ligus_tokens.rs`) and copied
  here unchanged with the JSON it came from (`scripts/sync-theme-tokens.sh`);
  `theme` builds every palette, light and dark, from it, and
  `tests/contrast.rs` checks the copy and every view's contrast.
  **`theme_file`** reads custom themes (`themes/NAME.toml`, the shared
  `ligus.theme.v1` format). **`termprobe`** asks the terminal for its
  background.
- **`config`**, **`theme`**, **`paths`**: the TOML config, the palettes,
  appearance and mono, and where the data dir and config file are.
- **`plain`**: the line-oriented client over the same `Backend`.
- **`setup`**: the guided setup flows on the shared engine
  (`ligus_runtime::setup`): lt's flow ids (the engine's seven, and a
  `device` hook listed only when `Backend::links_devices`), the view models
  both backends return, the secret and cancel types the reducer carries
  (`SetupSecrets`/`SecretBuf`: zeroized, redacted `Debug`, never
  serialized), folder completion, and `lt setup` (`setup::cli`).
  `app::setup` is the popup's pure state and keys; `ui::setup` draws it.
  Every run is a person's (`Driver::Person`); the engine refuses agents.

## The Backend contract

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    fn device(&self) -> DeviceInfo;
    fn events(&self) -> BoxStream<'static, Event>;
    async fn startup(&self) -> Result<StartupState>;
    async fn create_workspace(&self, your_name: &str, name: &str) -> Result<Created>;
    async fn inspect_invite(&self, invite: &str) -> Result<InviteCheck>;
    async fn join_workspace(&self, your_name: &str, invite: &str) -> Result<WorkspaceSummary>;
    async fn connect_deployment(&self, your_name: &str, spec: DeploymentSpec) -> Result<Created>;
    async fn inspect_join_request(&self, join_request: &str) -> Result<JoinRequestCheck>;
    async fn admit_box(&self, ws: &WorkspaceId, join_request: Option<&str>) -> Result<BoxAdmission>;
    async fn workflows(&self, ws: &WorkspaceId) -> Result<Vec<Workflow>>;
    async fn runs(&self, ws: &WorkspaceId) -> Result<Vec<Run>>;
    async fn start_run(&self, ws: &WorkspaceId, wf: &WorkflowId, input: &str) -> Result<Run>;
    async fn cancel_run(&self, ws: &WorkspaceId, run: &RunId) -> Result<Run>;
    async fn decide_gate(&self, ws: &WorkspaceId, run: &RunId, stage: &str, approve: bool) -> Result<Run>;
    // … chats, transcript, send_message, stop_turn, proposals, vote, request_apply,
    //   agents, handoffs, runtimes, local_runtimes, add_local_agent,
    //   hosts, checkout, set_checkout,
    //   memory, task_state, members, set_role, remove_member,
    //   lock_chat, clear_lockdown, notifications, settings …
    async fn answer_presence(&self, id: PresenceId, answer: PresenceAnswer) -> Result<()>;
}
```

The rules the UI relies on:

1. **Authorization belongs to the backend.** Views carry `Allowed::Yes` or
   `Allowed::No(reason)`: `Member::can_remove`, `Member::can_set_role` (one
   per role), `Proposal::can_vote`, `Lockdown::clear`,
   `WorkspaceSummary::can_invite` and `Workflow::can_start`. The UI crosses refused actions out and
   shows the reason. It never derives rank rules itself.
2. **Passkeys are prompts, not failures.** An action that needs presence
   (a role change, a removal, clearing a lockdown, a vote that requires it,
   creating the founder passkey) emits `Event::PresencePrompt` and waits for
   `answer_presence` before its future resolves. `Event::PresenceProblem`
   reports a fixable problem (an unreadable recovery file) without ending
   the ceremony; `Event::PresenceDone` ends it.
3. **Live updates come only through `events()`.** These are new messages,
   streaming deltas and completion, tool activity, agents thinking, run
   changes, and changes to proposals, workflows, memory, members, hosts,
   agents and chats. "Changed" events make the
   reducer reload that list. Results for a workspace that is no longer
   active are dropped.
4. **Errors are one line**, shown as toasts. `cancelled` is treated as
   quiet. One toast shows at a time, in the status bar's centre (never over
   a pane); every toast is also kept in the session's messages log (`!`),
   newest first and in full.

## Data dir and config

`lt` follows the `ligus` CLI's data-dir convention (an env var, else a dot
dir in `$HOME`, else a local dir) under its own names, so that it is always
a separate device (`src/paths.rs`):

| | `ligus` CLI | `lt` |
|---|---|---|
| flag | n/a | `--data-dir` |
| env | `LIGUS_DATA_DIR` | `LT_DATA_DIR` |
| default | `$HOME/.ligus` | `$HOME/.ligus-term` |
| no `$HOME` | `./ligus-data` | `./ligus-term-data` |

- `LIGUS_DATA_DIR` is never read: a shell exported for the CLI must not make
  `lt` the CLI's device.
- A data dir that overlaps the desktop app's (`<app data>/ai.ligus.desktop`,
  in either direction) is refused.
- So is one that overlaps the CLI's (`$LIGUS_DATA_DIR`, `$HOME/.ligus`, in
  either direction): two programs on one device identity would sign from the
  same per-device chain and equivocate.
- Both checks compare the paths as written and again with links resolved,
  so a symlink to another program's data dir is refused too.
- When wired, `lt` also calls the crates' `compat::claim_data_dir`, as the
  CLI does: it refuses directories holding older-app data and marks the
  dir as a Ligus one.

The config is `<platform config dir>/ligus-term/config.toml` (`--config`,
`LT_CONFIG`). It holds only the theme and key overrides. Device state
(workspace list, read markers, notification history, settings) lives in the
data dir, never in the config.

## Passkey presence in a terminal

The desktop's flow, which `lt` reuses unchanged on the browser side:

1. Bind `127.0.0.1:0` and draw a one-time nonce from the OS.
2. Write a private redirect page (0600 in a 0700 `<data dir>/presence/`)
   that forwards to
   `<policy.page_url()>#op=get&rp=<rp id>&c=<challenge>&cred=<ids>&k=<kind>&s=<summary>&p=<port>&n=<nonce>`.
   The nonce never appears in the `open` command line, which every local
   process can list.
3. Open that page in the system browser (`open` on macOS, `xdg-open`
   elsewhere). The page at `ligus.ai/presence`, or the workspace's own
   relying party, runs WebAuthn with `userVerification: "required"` and
   POSTs the result as a top-level form to `/<nonce>`.
4. Accept only `POST /<nonce>` with a capped header and body (4 KiB lines,
   64 headers, 16 KiB head, 64 KiB body). Anything else gets a 404 and is
   ignored. Wait up to 180 s.
5. The backend checks the returned `clientDataJSON` challenge is its own;
   the fold verifies the assertion again on every peer.

`src/backend/presence.rs` implements steps 1–4 with tokio (unit-tested
against a fake browser that reads the redirect page and posts back). The
terminal sheet shows the kind, the summary and the digest's short id, and
offers two ways out:

- **Recovery-words file** (`f`, confirm prompts only): the path to a file
  with the 24 words, the file the CLI's `--recovery-words-file` takes. `lt`
  refuses a file that others can read. The real backend signs the very
  challenge with `RecoveryKey::from_words(..)?.assert(&challenge)`.
- **Cancel** (`esc`): stops the wait. The action fails with `cancelled`.

Both can be chosen while the browser is already open: the loopback wait is
cancelled.

## Wiring to the Ligus crates

### Dependency

**Today** `lt` depends on the crates as git dependencies on
`https://github.com/langstroth-ai/ligus`, pinned to its `v0.1.0` release tag:

```toml
[dependencies]
ligus-core    = { version = "0.1.0", git = "https://github.com/langstroth-ai/ligus", tag = "v0.1.0" }
ligus-runtime = { version = "0.1.0", git = "https://github.com/langstroth-ai/ligus", tag = "v0.1.0", default-features = false }
```

- **Tags only.** lt depends on a released ligus tag; a newer ligus release
  moves the pin to its tag. Never depend on a branch.
  While code-match approval (Ligus 0.4) is unreleased, the pin is an exact
  commit (`rev = ...`) of the ligus repo's `feat/codematch-protocol`
  branch, never the branch itself; it moves to the `v0.4.0` tag once that
  is cut.
- **Features.** `ligus-runtime` is built without its `p2p` feature (no iroh)
  for now; the dev-dependency adds `test-relay` for the tests.
- **`version = "0.1.0"`** sits next to `git` so cargo-deny doesn't count
  the git dependencies as wildcards; `deny.toml` allows the git source.
- **Toolchain.** The crates are edition 2021 with `rust-version = 1.91`,
  which is why `lt` declares 1.91 too. Mixing editions across crates is
  fine.
- **Lockfile.** `lt`'s own `Cargo.lock` resolves the crates' transitive
  dependencies independently of the ligus repo's lock; it was seeded from
  that lock so they resolve to the versions the ligus repo tests with. Run
  the 7-day age check (`scripts/check-dep-age.py`) and the test suite after
  every bump.
- **The fake stays.** It becomes `lt --demo` and the test backend. The real
  backend is the default.

### Opening the device

At startup (`RealBackend::open(data_dir)`), mirroring the CLI's
`open_service`:

1. `compat::claim_data_dir(data_dir, home)`
2. `secret_store::init_for_data_dir(data_dir, None)`
3. `IdentityStore::new(data_dir, StoreKeyVault::for_data_dir(data_dir))`,
   then `.load()`. `None` means first run: `bootstrap(name, handle,
   device_name)` happens when the person creates or joins.
4. `EventStore::open(<data dir>/<database file>)`
5. `ChatService::new(store, device_id, device_kp, account_kp, actor)`
6. `svc.set_agent_vault(Box::new(StoreKeyVault::for_data_dir(data_dir)))`
7. `svc.set_presence_provider(Some(Arc::new(TermPresence { … })))`

### Method mapping

Paths are inside the crate named in each row, at the ligus `v0.1.0` tag.
`ChatService` is `runtime/src/chat_service.rs`.

| `Backend` method | Crate API | Notes |
|---|---|---|
| `device` | `IdentityStore::load` → `StoredIdentity` | Safety code as the desktop derives it; secret-store backend name from `SecretStore::backend()` |
| `events` | none: `RealBackend` owns a `tokio::sync::broadcast` | `ChatService` has no callbacks. Sources are listed below |
| `startup` | `IdentityStore::load`; `lt`'s workspace list in the data dir | The desktop keeps its workspace list in app settings; there is no crate API |
| `create_workspace` | `IdentityStore::bootstrap`; `ChatService::ensure_workspace_config(ws)`; founder passkey via `register_passkey(ws, cred, &prompt)` with a browser `create` ceremony; `RecoveryKey::generate()` → `.words()`, registered with `register_passkey(ws, key.credential(account, "Recovery key"), …)` | Room and key are random tokens, as the desktop's `create_workspace` makes them. No server needed |
| `inspect_invite` | `workspace_invite::decode_workspace_invite`; `WorkspaceConn::signed_by` (the founder's device signature, `invite_sig`) against the devices `directory_trust::co_members` says are revoked; `dm::check_founder` against `dm::known_fingerprints` | The desktop's `inspect_invite` / `invite_trust`. A DM invite is refused |
| `join_workspace` | refused for a workspace this device already holds (an invite never rewrites its relay, key or founder pin); the same check (a `Mismatch` is refused, as the desktop's drop-box accept does), then `store().set_join_invite(ws, inv)`, the founder pin, `SyncEngine::catch_up`, `ensure_self_member` | |
| `connect_deployment` | `workspace_invite::usable_relay_url`; a `SyncEngine::catch_up` to check the relay holds no config log for the room; then as `create_workspace`, with the deployment's relay, room and key (and relay token, kept with the connection secrets) | A box never founds a workspace (`admit::prepare_box` in the CLI), so the first person does |
| `inspect_join_request` | `box_admission::JoinRequest::decode`, its `fingerprint()` and label; no write, no presence | lt shows (TUI) or prints (plain) this before `admit_box`, so the Admin compares it with the box's terminal before the passkey prompt |
| `admit_box` | Admin+; `box_admission::JoinRequest::decode`; `ChatService::issue_box_admission(ws, ttl, request)` (presence); a signed invite (`sign_invite`) with `inv` and `ek`; `box_admission::with_redeem_key`; `JoinRequest::seal` when there is a request | As `ligus admit issue` (`crates/ligus-cli/src/admit.rs`, `serve::admission_code`) and the desktop's `admit_box`. TTL 1 h sealed, 24 h bearer |
| `confirm_recovery_saved` | device-local flag | |
| `workspaces` | `lt`'s workspace list | Relay URL and room in `workspaces.json`; secrets in the secret store |
| `create_invite` | `ChatService::issue_invite(ws)` (Admin+), then sign and encode | Refused when the workspace has no relay (`workspace-no-relay`) |
| `chats` | `store().list_session_ids()` minus `workspace_config_session_id(ws)`; `list_channels(ws)` | |
| `create_chat` | `ChatService::create_chat(title, ws, runtime_id)` | |
| `transcript` | `ChatService::load(sid)?.messages` | |
| `send_message` | `post_user_message(sid, ws, body)`; then `turn_dispatch::dispatch_target` and `runs_here_linked`; a turn that runs here is claimed (`claim_turn`) and streamed (`begin_assistant_message`, `append_chunk`, `complete_assistant_message`). A top-level turn deferred to an owner whose host isn't online elsewhere arms the **deferred-claim fallback**: after `DEFERRED_CLAIM_TIMEOUT` (25 s), if `turn_dispatch::deferred_turn_unanswered` still holds and the host still isn't online, claim it and answer here | The desktop's `arm_deferred_claim_fallback` / `run_deferred_claim_fallback`. The relay-sync gate never arms it (the sending device does) |
| `mentionables` | `session.members`, `session.workspace_agents`, plus `@primary` and `@all` | |
| `mark_read` | device-local read markers | |
| `lock_chat` | `self_medicate(ws, chat, None, &[(LockdownSignal::Manual, "manual")])` | Viewer and above |
| `clear_lockdown` | `clear_lockdown(ws, chat_id, raise_ids)` | Human Contributor and above, with presence. `Lockdown::clear` is computed as the desktop's `get_chat_lockdown` does |
| `proposals` | `load(sid)?.proposals` across the workspace's chats (`ActionProposal`) | `kind`: `workflow` when it carries a definition, `gate` when a run's node points at it, else decision or change |
| `proposal_diff` | `ActionProposal::diff` (a unified diff) | `lt` parses it into files and hunks |
| `vote` | `vote_on_proposal(sid, ws, id, approved)` | Presence when the rules require it |
| `request_apply` / `withdraw_apply` | `request_apply(sid, ws, id, host_device_id, expected_base_commit)` / `withdraw_apply_request` | When `lt` is itself a host: `sync_service::apply_tick(&svc, ws, founder, repo_for)`, which lists and judges each request under `ws`'s founder pin (the service is shared by every workspace) |
| `agents` | `session.workspace_agents` (`WorkspaceAgent`); busy when one of the last 20 messages of a chat streams for it under ten minutes old (serve's `LigusSnapshot` rule); offline when its host's beat is older than `HOST_ONLINE_WINDOW_SECS`; `doing` from its active task-state slot | |
| `handoffs` | `session.flow.handoffs` (`HandoffCard`) across chats, newest first | |
| `set_agent_runtime` | `add_agent` (upsert) with the new `runtime_id` | Per chat: `set_session_runtime` |
| `runtimes` | `list_workspace_runtimes(ws)`, then the device-local runtimes (`local:…`) the workspace's agents name | |
| `local_runtimes` | `backend::real::detect`: `cli_detect::find_on_path` and a bounded `--version` for `claude` and `codex`, `cli_detect::detect_cli_agents(env, false)` for `agy`, `copilot` and a legacy `gemini`, a 300 ms connect probe then `OllamaClient::list_models` at `RealConfig::ollama_url`, and the presence of `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY`. `used_by`: agents hosted here naming it, or naming a workspace runtime of its kind | Never sends a prompt (no `agy models` either), never opens a keychain (sign-in from a file's existence or a variable's presence), never copies a secret (the environment is reduced to "set" markers). The Claude Code, Codex and Ollama checks repeat the desktop's `detect_environment` / `detect_providers` and `ligus setup` (outside the library); TODO: move them into `cli_detect` and call that |
| `add_local_agent` | `add_agent(config, ws, WorkspaceAgent { runtime_id: "local:…", owner_actor_id: you, access: "owner", host_id })`: the desktop's `add_agent`, one signed `AgentUpserted` | `host_id` is this device for an Admin or Owner (pinning needs Admin), else empty (its owner's device). Names: the desktop's reserved words, no agent or member name twice. At turn time `turns::runtime_for` resolves `local:<provider>[/<model>]` to the CLI's program, Ollama at `ollama_url`, or a hosted API with its key read from this device's environment |
| `hosts` | `list_hosts(ws)`; online and last seen from `HostPresence`, fed by `host_presence::fetch_host_beats` every `beat_every` (30 s); busy and agents from `agents` | `lt` registers itself with `ensure_host` / `touch_host` and publishes its own beat (`publish_host_beat`). Beats carry no repo coverage yet, so `repos` is empty |
| `checkout` / `set_checkout` | device-local (`lt-state.json`), checked to be a git checkout | `apply_tick`'s `repo_for` uses it for every chat. The release has no synced repo bindings yet (PR #127, repos v2, isn't merged) |
| `workflows` | `session.workflow_definitions` across chats; `workflow::validate` for `can_start` | Definitions belong to a chat |
| `runs` | `session.workflow_runs` across chats; the driver is the `LeaseScope::WorkflowRun` lease holder's host label | |
| `start_run` / `cancel_run` / `decide_gate` | `workflow::new_run`, `upsert_workflow_run`, `claim_lease(.., WorkflowRun, run, 1, DEFAULT_LEASE_MS, Initial)`, then the driver below; a gate is `vote_on_proposal` on its proposal | See "Workflow runs" |
| `memory` / `task_state` | `ChatService::memory(ws)` (`MemoryState::live`, `open_proposals`); `flow.tasks` (`TaskSlot`, not `stale`) across chats | Read-only, as serve's `/v1/memory` and `/v1/chats/{id}/task-state` |
| `members` | `session.members` (`WorkspaceMember`, `WorkspaceRole`) plus `authz::member_action_refusal(roster, config, author, &MemberAction::Remove{..} / SetRole{..})` for each member and action | `None` → `Allowed::Yes`; `Some(reason)` → `Allowed::No(reason)`. The UI shows the crates' wording |
| `set_role` / `remove_member` | `set_member_role(_, ws, id, role)` / `remove_member(_, ws, id)` | Presence |
| `notifications` | `mentions::message_mentions` / `parse_mentions(..).you` on synced messages | History is device-local |
| `settings` / `set_setting` | device-local | |
| `answer_presence` | resolves `TermPresence`'s pending ceremony | See below |

**Event sources** for `RealBackend::events()`:

- **Sync.** A loop like the desktop's: `SyncEngine::sync_once`, driven by
  `RelayClient::open_event_stream` (SSE) with a polling fallback, or by
  `PeerSync` over iroh. After each round, diff the projections and emit
  `MessageAdded`, `ChatsChanged`, `ProposalChanged`, `MembersChanged`,
  `HostsChanged` and `AgentsChanged`, plus `Sync` status.
- **Local turns.** Chunks as `MessageDelta`, tool calls as `ToolActivity`,
  and completion as `MessageDone`.
- **Admissions.** After a sync round that pulled nothing (caught up), an
  Admin's device closes the box admissions that settled
  (`close_settled_admissions`), as the desktop does.
- **Presence channel.** Per workspace with a relay, every `beat_every`:
  publish this host's beat and the lease beat of each run this device
  drives (`lease_presence::LeaseBeat`, signed with `sign_lease_beat`),
  read every host's beat (kept per workspace: a beat counts only in the room it came from); a changed online set emits `HostsChanged` and
  `AgentsChanged`. People's beats (`human_presence`) are never read.
- **Runs.** The translator emits `RunChanged` when a session's runs change
  (synced or local), and the driver emits one after each write it makes;
  definition changes are `WorkflowsChanged`, memory and task state
  `MemoryChanged`.
- **Presence.** `TermPresence` emits `PresencePrompt`, `PresenceProblem`
  and `PresenceDone`.

### Agent turns and the sandbox

A turn runs as on every Ligus host. `turns::guard_turn`
(`turn_engine::decide_gate_with`) applies the author-role gate
(`author_gate::gate_turn`, with the role the trigger's author held when they
posted), the shell rule (the shell is the workspace Owner's, from
`trigger_shell_role`, or an Owner's grant to the agent; the signed path
grants apply per `LIGUS_SANDBOX_GRANTS`), the chat's sandbox envelope
(`sandbox::envelope_for_session` and `sandbox::enforce`: a lockdown or dearth
mode narrows egress or refuses a runtime it can't confine) and this host's
OS sandbox (`executor::turn_allowed`). The repo gate is
`turn_engine::repo_checkout`: a chat with a workspace repo runs only in this
device's checkout of it, any other chat in the checkout bound for the
workspace.

The turn itself runs in the runtime's shared engine (`turn_engine::run`), the
same one the desktop and `ligus worker` / `ligus serve` run, with `lt` as its
host (`turns::TermHost`): the gate's `TurnPolicy` and the host's executor are
inputs, so every child (the keyless Claude Code fallback, any CLI runtime,
`run_command`) is spawned confined. A runtime that runs the tool loop
(Custom, a local gateway, Ollama; `LT_AGENT_TOOLS=0` turns it off) gets
Ligus's built-in file tools (on a private endpoint), the folded Ligus tools
(propose, hand off, ask, read) and, with `LT_RUN_COMMAND=1` and the shell,
`run_command`. Its edits land in a per-turn worktree of the checkout, cut on
the first write, and come back as a FileDiff proposal; a hand-off runs the
receiver here when it is this device's to answer. Tool calls show live as
`ToolActivity` under the reply. `RealBackend::open` registers `lt`'s data dir
with `executor::configure_host`, so no agent child can read its keys,
database or secrets, and on Linux `main` dispatches the sandbox's pre-exec
helper before anything else.

### Workflow runs

The crates hold the records and the scheduling rules; the driver loop
lives in each client (the desktop's `app/src/workflows.rs`). `lt`'s is
`src/backend/real/workflows.rs` and follows the desktop's:

1. **Start**: `new_run`, `upsert_workflow_run`, claim the run's lease at
   epoch 1. Every later write carries `lease_epoch` (the fencing token) and
   the presence loop beats the lease, so a desktop's recovery doesn't take
   the run over while `lt` drives it. If `lt` stops, the lease lapses and a
   desktop in the workspace takes the run over (`recover_workflow_runs`).
2. **Each round** (under the service locks, no awaits): stop if the lease
   moved or the run settled elsewhere; fold gate proposals
   (`apply_gate_outcome`), `propagate_skips`, `derive_run_status`; dismiss
   gate proposals a reset or settled run no longer waits on; open a
   `Decision` proposal (no author, `required_approvals`) for each ready
   gate; mark one ready stage per agent `Running` and persist.
3. **Stages**: a stage whose responder runs here (`runs_here_linked`) posts
   its prompt (`post_workflow_stage_prompt`), claims it and runs it with
   the same turn runner and sandbox policy as any reply. Any other stage
   posts the prompt with an `@mention`, so the agent's host answers it like
   any mention (a `ligus worker` drains `pending_turns`); the driver waits
   (up to `stage_timeout`, 300 s) for the reply whose
   `reply_to_message_id` is the prompt.
4. **Gates** are decided by votes from anyone (lt's `decide_gate`, the
   desktop, the phone); a vote and every synced change wake the driver.
5. **Cancel** sets a flag the next round honours (Skipped, Canceled).

Not done here: the desktop's idle-host offer (taking a stage from a busy
or offline host), throttle waits, stage hand-off and wait records in the
flow layer, and taking over a run whose driver went away.

### Threading

`ChatService` is synchronous; the desktop holds it in a `Mutex`.
`RealBackend` holds `Arc<std::sync::Mutex<ChatService>>` and runs each call
in `tokio::task::spawn_blocking`.

`PresenceProvider::assert` is called from inside a `ChatService` method and
blocks that blocking-pool thread. `TermPresence` does the following:

1. Sends `PresencePrompt` on the broadcast channel.
2. Waits on a channel for the UI's answer.
3. For `Browser`, runs `presence::hand_off` on a runtime handle
   (`Handle::block_on`) with the request's policy:
   `get_fragment(rp_id, b64url(challenge), credential_ids, kind, summary)`
   against `policy.page_url()`. It then checks the returned challenge and
   builds the `PresenceAssertion`.
4. For `RecoveryWordsFile`, signs the challenge with the recovery key.

The service mutex is held during the ceremony, up to 180 s. Reads must not
queue behind it: serve list and transcript reads from a second read-only
`EventStore` handle, or snapshot after each write.

### Key storage

`lt` uses the same secret-store backends as the CLI, through
`secret_store::init_for_data_dir(lt_data_dir, None)`:

| Backend | Where |
|---|---|
| macOS keychain | A dedicated keychain file `<data dir>/secrets/ligus.keychain-db`. Its unlock password is a login-keychain item (service `ai.ligus.secrets.unlock`, account derived from the keychain path, so `lt`'s item is separate from the desktop's and the CLI's) |
| Secret Service | Linux desktop, through `secret-tool` |
| `systemd-creds` | Headless Linux (TPM2 when present) |
| `File0600` | Owner-only files under the data dir. The fallback, with a warning; on macOS only when chosen |

`LIGUS_SECRET_BACKEND` picks one explicitly, as for the CLI.

- **Identity seeds** go through `StoreKeyVault` (scope `seed`).
- **Other secrets.** `lt`'s connection secrets (relay tokens, workspace
  passphrases, delivered epoch keys) use their own scope, `term`, mirroring
  the CLI's `cli`. `config.toml` and `workspaces.json` never hold a
  secret.
- **macOS signing.** The keychain's unlock item trusts only the binary that
  created it. A bare `lt` binary isn't an app bundle, so it is pinned by
  code hash, and every new build asks once for the login keychain password.
  Release builds should be signed (Developer ID, identifier `ai.ligus.term`).
  The crates' access-list code needs a way to name that requirement for a
  non-desktop binary (see Gaps).

### What is wired

`src/backend/real/` implements the whole `Backend` over the crates:

| Module | What |
|---|---|
| `mod.rs` | `RealBackend::open`: `claim_data_dir`, the secret store (scope `term`, signing identifier `ai.ligus.term`), the identity (made on the first create, join or connect), a write `ChatService` with the agent vault and the presence provider, and a read-only `ChatService` on its own store connection. Every method of the trait |
| `deploy.rs` | The invite check, founding a deployment's workspace, box admission |
| `workflows.rs` | Starting, driving, cancelling runs; run views; the lease |
| `term_presence.rs` | The `PresenceProvider`: prompt events, the loopback browser hand-off for `get` and `create`, the recovery-words file |
| `turns.rs` | The local-send and relay-sync gates (`ligus_runtime::turn_dispatch`), the claim, the deferred-claim fallback, and the turn runner (workspace runtime, else the keyless Claude Code CLI) |
| `watch.rs` | The change-feed translator, the sync loop per workspace (`ligus_runtime::sync_service::SyncLoop`), the presence loop (host and lease beats), and the host loop (durable heartbeat, apply on host) |
| `views.rs` | Projections to view models; member refusals from `authz::member_action_refusal` |
| `local.rs` | Device-local state: read markers, notifications, settings, checkouts |
| `setup_ops.rs` | The guided setup engine's `SetupOps` (`ligus_runtime::setup`, `Surface::Term`): agents on lt's `local:` runtimes (a new runtime is only its id), tools, folders (`repos::bind`, and lt's checkout when none is set), the relay (the saved workspace, then its sync loop restarted); a box, a phone and an invite hand off to a person |
| `mcp.rs` | This device's MCP servers per workspace (`mcp-servers.json`, `mcp-reach.json`, tokens in the secret store bound to their origin) and the ones a turn may reach (`TermHost::mcp_servers`) |

Working for real, and covered by `tests/real_backend.rs` and
`tests/orchestration.rs`: first run, create, invites, join by invite with
the founder-signature check, founding a deployment's workspace on its
relay, admitting a box (sealed and bearer), a worker-like box answering
its agent's stages, workflow runs end to end (a remote stage, a gate
approved from `lt`, a local stage), live run events, host beats (online,
busy, agents per host), the deferred-claim fallback, chats, streamed agent
replies through both dispatch gates, proposals and diffs, votes, apply on
a host's checkout and the checkout binding, people, lockdowns, mention
notifications, plain mode over the real backend, and a device reopened
from disk.

Not wired yet:

- **Peer-to-peer sync** (iroh): relay only.
- **MCP servers for coding CLIs.** This device's tools (Connect a tool,
  `real/mcp.rs`) reach turns that run the tool loop; Claude Code and the
  other CLIs aren't handed them on their command line (the desktop does).
- **Cascades.** A reply that hands off to another agent isn't run here
  (the desktop's cascade loop); a worker hosting that agent answers it.
- **Run takeover.** `lt` drives the runs it starts; it doesn't take over a
  run whose driver went away (the desktop does), nor offer to run a stage
  queued on a busy host.
- **Repos per checkout.** One checkout per workspace (device-local); repos
  v2 (PR #127) isn't on the release branch yet, so there are no synced
  workspace repos to bind one by one, and beats carry no repo coverage.
- **Writing memory.** Memory and task state are read-only here.
- **Relay tokens** are stored per workspace when given at connect; there is
  no `ligus auth login` flow to refresh one.

### Gaps in the crates

Wiring `lt` surfaced logic that lived in the desktop app or the CLI binary.
PR #121 (`feat/runtime-for-clients`, merged into `release/v0.1.0`) moved it into
`ligus-runtime`, and the desktop and the CLI now use
the shared copy:

1. **Workspace invite codec**: `workspace_invite` (`WorkspaceConn`,
   `encode_workspace_invite`, `decode_workspace_invite`,
   `invite_relay_problem`). Done.
2. **Workspace list and connection settings**: `connection_settings`
   (`ConnectionSecrets` for the CLI's `cli` scope, `WorkspaceList` for
   clients with several workspaces). Done.
3. **Turn dispatch**: `turn_dispatch` (`turn_runs_here`,
   `runs_here_linked`, `dispatch_target`, the answered and deferred
   checks), and `sync_service` (`SyncLoop`, `sync_round`, `apply_tick`).
   The turn runner is `turn_engine` (tools, worktrees, proposals,
   hand-offs), shared by the desktop, the worker and `lt`; context windows
   and summaries are still the desktop's.
4. **Lockdown view**: `lockdown_view` (`can_clear`, `chat_lockdown`). Done.
5. **Live updates**: `ChatService::subscribe` (a broadcast of every
   append; sync loops publish what they pulled to the same feed). Done.
6. **Serverless across devices.** Still open: joining needs a relay.
7. **Keychain trust for other binaries**:
   `secret_store::set_binary_identifier`. Done.
8. **Workflow driver.** Still open: the run loop (scheduling rounds, stage
   dispatch, remote waits, lease handling, takeover) exists once in the
   desktop and once in `lt` (`real/workflows.rs`); serve and workers don't
   drive runs. It belongs in the runtime with the agent-turn engine.
9. **Invite trust.** Small and composed from crate parts in `lt`
   (`signed_by`, `co_members`, `dm::check_founder`); the desktop's
   `invite_trust` is the same few lines. Could move next to `dm`.

## Testing

- **Screens:** `tests/screens.rs` (insta snapshots, text captures in
  `docs/screens/`).
- **Reducer:** `tests/reducer.rs`.
- **Keys and config:** `tests/keymap.rs`.
- **The binary:** `tests/cli.rs`.
- **Unit tests:** the fake backend, the presence hand-off, the data-dir
  rules, text editing and plain mode.
- **Real backend:** `tests/real_backend.rs` and `tests/orchestration.rs`
  (helpers in `tests/support/`). Devices with temporary data dirs and
  `LIGUS_SECRET_BACKEND=file`, syncing through the crates' in-process test
  relay (`ligus_runtime::test_relay`, feature `test-relay`), a fake
  OpenAI-compatible model server, a software passkey
  (`ligus_core::presence::soft`) playing the browser, and a worker-like box
  (a `ChatService` admitted with a sealed admission, answering its agent's
  mentions as `ligus worker` does). Tests never touch the login keychain
  or a real relay.
- **Plain mode:** unit tests in `src/plain.rs` (records and exit codes on
  the fake), `tests/cli.rs` (the binary's one-shot exit codes), and
  `tests/orchestration.rs` (one-shot commands over the real backend).
