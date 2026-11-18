# lt architecture

`lt` is a full Ligus client in the terminal (Ligus is short for
*ligustica*). It is its own device: its own keys and data dir, joining
workspaces and signing events like the desktop app, and working without a
server. This document covers how the app is put together today and how it
gets wired to the Ligus Rust crates.

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
  `BINDINGS`, feeds resolution, the help overlay and `--print-config`).
- **`config`**, **`theme`**, **`paths`**: the TOML config, dark/light/mono
  themes, and where the data dir and config file are.
- **`plain`**: the line-oriented client over the same `Backend`.

## The Backend contract

```rust
#[async_trait]
pub trait Backend: Send + Sync {
    fn device(&self) -> DeviceInfo;
    fn events(&self) -> BoxStream<'static, Event>;
    async fn startup(&self) -> Result<StartupState>;
    async fn create_workspace(&self, your_name: &str, name: &str) -> Result<Created>;
    async fn join_workspace(&self, your_name: &str, invite: &str) -> Result<WorkspaceSummary>;
    // … chats, transcript, send_message, proposals, vote, request_apply,
    //   agents, runtimes, hosts, members, set_role, remove_member,
    //   lock_chat, clear_lockdown, notifications, settings …
    async fn answer_presence(&self, id: PresenceId, answer: PresenceAnswer) -> Result<()>;
}
```

The rules the UI relies on:

1. **Authorization belongs to the backend.** Views carry `Allowed::Yes` or
   `Allowed::No(reason)`: `Member::can_remove`, `Member::can_set_role` (one
   per role), `Proposal::can_vote`, `Lockdown::clear` and
   `WorkspaceSummary::can_invite`. The UI crosses refused actions out and
   shows the reason. It never derives rank rules itself.
2. **Passkeys are prompts, not failures.** An action that needs presence
   (a role change, a removal, clearing a lockdown, a vote that requires it,
   creating the founder passkey) emits `Event::PresencePrompt` and waits for
   `answer_presence` before its future resolves. `Event::PresenceProblem`
   reports a fixable problem (an unreadable recovery file) without ending
   the ceremony; `Event::PresenceDone` ends it.
3. **Live updates come only through `events()`.** These are new messages,
   streaming deltas and completion, tool activity, typing, and changes to
   proposals, members, hosts, agents and chats. "Changed" events make the
   reducer reload that list. Results for a workspace that is no longer
   active are dropped.
4. **Errors are one line**, shown as toasts. `cancelled` is treated as
   quiet.

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
- Pointing `lt` at the CLI's dir gets a warning.
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

The crates are being renamed (to `ligus-core`, `ligus-runtime`,
`ligus-proto`, `ligus-forge`). After the rename `lt` depends on them as git
dependencies pinned to a release tag:

```toml
[dependencies]
ligus-core    = { git = "https://github.com/langstroth-ai/ligus", tag = "v0.1.0" }
ligus-proto   = { git = "https://github.com/langstroth-ai/ligus", tag = "v0.1.0" }
ligus-runtime = { git = "https://github.com/langstroth-ai/ligus", tag = "v0.1.0", features = ["p2p"] }
```

- **Tag first.** The reference tree (branch `release/v0.1.0`) has no tags
  yet and its crates still say `0.1.0`. Cut the tag before wiring, and
  never depend on a branch.
- **Toolchain.** The crates are edition 2021 with `rust-version = 1.91`,
  which is why `lt` declares 1.91 too. Mixing editions across crates is
  fine.
- **cargo-deny.** Allow the git source in `deny.toml`
  (`[sources] allow-git`).
- **Lockfile.** `lt`'s own `Cargo.lock` resolves the crates' transitive
  dependencies independently of the ligus repo's lock. Run the 7-day age
  check (`scripts/check-dep-age.py`) and the test suite after every bump.
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

Paths are inside the crate named in each row, on `release/v0.1.0`.
`ChatService` is `runtime/src/chat_service.rs`.

| `Backend` method | Crate API | Notes |
|---|---|---|
| `device` | `IdentityStore::load` → `StoredIdentity` | Safety code as the desktop derives it; secret-store backend name from `SecretStore::backend()` |
| `events` | none: `RealBackend` owns a `tokio::sync::broadcast` | `ChatService` has no callbacks. Sources are listed below |
| `startup` | `IdentityStore::load`; `lt`'s workspace list in the data dir | The desktop keeps its workspace list in app settings; there is no crate API |
| `create_workspace` | `IdentityStore::bootstrap`; `ChatService::ensure_workspace_config(ws)`; founder passkey via `register_passkey(ws, cred, &prompt)` with a browser `create` ceremony; `RecoveryKey::generate()` → `.words()`, registered with `register_passkey(ws, key.credential(account, "Recovery key"), …)` | Room and key are random tokens, as the desktop's `create_workspace` makes them. No server needed |
| `join_workspace` | decode the workspace invite, `store().set_join_invite(ws, inv)`, then sync with `SyncEngine`, or `RelayClient::join_via_invite` | The invite codec lives in the desktop app and the CLI today (see Gaps) |
| `confirm_recovery_saved` | device-local flag | |
| `workspaces` | `lt`'s workspace list | Relay URL and room in `workspaces.toml`; secrets in the secret store |
| `create_invite` | `ChatService::issue_invite(ws)` (Admin+), then sign and encode | Refused when the workspace has no relay (`workspace-no-relay`) |
| `chats` | `store().list_session_ids()` minus `workspace_config_session_id(ws)`; `list_channels(ws)` | |
| `create_chat` | `ChatService::create_chat(title, ws, runtime_id)` | |
| `transcript` | `ChatService::load(sid)?.messages` | |
| `send_message` | `post_user_message(sid, ws, body)`; then dispatch: `parse_mentions(body, &session)`, `turns_for`, `pending_mentions`; for turns this device runs, `begin_assistant_message`, `append_chunk`, `complete_assistant_message` | Each chunk also goes out as `Event::MessageDelta`. The two dispatch gates (local send and relay-synced) must agree: see Gaps |
| `mentionables` | `session.members`, `session.workspace_agents`, plus `@primary` and `@all` | |
| `mark_read` | device-local read markers | |
| `lock_chat` | `self_medicate(ws, chat, None, &[(LockdownSignal::Manual, "manual")])` | Viewer and above |
| `clear_lockdown` | `clear_lockdown(ws, chat_id, raise_ids)` | Human Contributor and above, with presence. `Lockdown::clear` is computed as the desktop's `get_chat_lockdown` does |
| `proposals` | `load(sid)?.proposals` across the workspace's chats (`ActionProposal`) | |
| `proposal_diff` | `ActionProposal::diff` (a unified diff) | `lt` parses it into files and hunks |
| `vote` | `vote_on_proposal(sid, ws, id, approved)` | Presence when the rules require it |
| `request_apply` / `withdraw_apply` | `request_apply(sid, ws, id, host_device_id, expected_base_commit)` / `withdraw_apply_request` | When `lt` is itself a host: `apply_host::pending(&svc, ws)` and `apply_host::fulfil(&svc, &job, repo)` in a loop, as the CLI worker's `apply_tick` does |
| `agents` | `session.workspace_agents` (`WorkspaceAgent`) | |
| `set_agent_runtime` | `add_agent` (upsert) with the new `runtime_id` | Per chat: `set_session_runtime` |
| `runtimes` | `list_workspace_runtimes(ws)` | |
| `hosts` | `list_hosts(ws)` plus `host_presence::fetch_host_beats` for online/last seen | `lt` registers itself with `ensure_host` / `touch_host` and publishes its own beat |
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
- **Typing.** `human_presence::fetch_human_beats` + `typing_in`; publish
  our own with `publish_human_beat`.
- **Presence.** `TermPresence` emits `PresencePrompt`, `PresenceProblem`
  and `PresenceDone`.

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
  the CLI's `cli`. `config.toml` and `workspaces.toml` never hold a
  secret.
- **macOS signing.** The keychain's unlock item trusts only the binary that
  created it. A bare `lt` binary isn't an app bundle, so it is pinned by
  code hash, and every new build asks once for the login keychain password.
  Release builds should be signed (Developer ID, identifier `ai.ligus.term`).
  The crates' access-list code needs a way to name that requirement for a
  non-desktop binary (see Gaps).

### Gaps in the crates to close at the rename

Wiring `lt` surfaced logic that lives in the desktop app or the CLI binary
rather than in a library crate. Each item should move into
`ligus-runtime`, so that `lt`, the CLI and the desktop share one copy:

1. **Workspace invite codec.** Encoding and decoding live in the desktop
   app and the CLI.
2. **Workspace list and connection settings.** Relay, room, founder pin:
   app-level today.
3. **Turn dispatch.** The gate that decides whether a turn runs on this
   device, the turn runner, the sync loop and the apply loop are in the
   desktop app and in the CLI's worker and serve modules. The local-send
   gate and the relay-sync gate must stay identical (solo always answers).
4. **Lockdown view.** `can_clear` and its signals are computed in the
   desktop's command layer.
5. **Live updates.** `ChatService` exposes no subscription. A small
   observer hook would save every client from diffing projections.
6. **Serverless across devices.** Today peer sync carries only the current
   room when a relay is configured, and invites are refused without a
   relay. A workspace works without a server on one device; joining from a
   second device needs a relay until relay-less invites over iroh land.
7. **Keychain trust for other binaries.** Make the designated requirement
   configurable, so that a signed `lt` gets the same protection as the
   desktop app.

## Testing

- **Screens:** `tests/screens.rs` (insta snapshots, text captures in
  `docs/screens/`).
- **Reducer:** `tests/reducer.rs`.
- **Keys and config:** `tests/keymap.rs`.
- **The binary:** `tests/cli.rs`.
- **Unit tests:** the fake backend, the presence hand-off, the data-dir
  rules, text editing and plain mode.
- **Real backend.** When it lands it gets its own tests against a
  temporary data dir with `LIGUS_SECRET_BACKEND=file` and an in-memory
  `EventStore`. Tests never touch the login keychain or a real relay.
