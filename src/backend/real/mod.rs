//! The real [`Backend`]: `lt` as its own Ligus device, over the Ligus crates.
//!
//! Startup mirrors the `ligus` CLI's `open_service`, with `lt`'s own data
//! dir and secret-store scope (`term`):
//!
//! 1. `compat::claim_data_dir`, then the secret store for the data dir;
//! 2. the identity (`IdentityStore`), created on the first create or join;
//! 3. the event store and a `ChatService` for writes, with this device's
//!    agent vault and the terminal's passkey presence provider
//!    ([`term_presence`]);
//! 4. a second, read-only `ChatService` on its own store connection, so a
//!    write waiting on a passkey (up to 180 s, holding the write lock) never
//!    blocks a read.
//!
//! `ChatService` is synchronous: every call runs in `spawn_blocking`. Live
//! updates come from its change feed ([`watch`]); turns this device answers
//! run in [`turns`], behind the same dispatch gates as the desktop
//! (`hive_runtime::turn_dispatch`).

mod local;
mod term_presence;
mod turns;
pub mod views;
mod watch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use hive_core::presence::{PasskeyCredential, PresencePolicy, RecoveryKey, b64url};
use hive_core::{Timestamp, workspace_config_session_id};
use hive_runtime::chat_service::{ChatError, PresenceRequest};
use hive_runtime::connection_settings::{SavedWorkspace, WorkspaceList};
use hive_runtime::secret_store::{SecretStore, StoreKeyVault};
use hive_runtime::workspace_invite::{WorkspaceConn, decode_workspace_invite, encode_workspace_invite, invite_relay_problem, random_token};
use hive_runtime::{ChatService, EventStore, IdentityStore, StoredIdentity};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::model::*;
use super::presence::{self, Opener};
use super::{Backend, BackendError, Event, Result, err};
use local::LocalState;
pub use local::SECRET_SCOPE;
use term_presence::{Provider, TermPresence};

/// The database file under the data dir (the CLI's name).
pub const DB_FILE: &str = "hive.db";
/// The code-signing identifier of a release `lt`.
pub const SIGNING_IDENTIFIER: &str = "ai.ligus.term";

/// How the real backend runs.
#[derive(Clone)]
pub struct RealConfig {
    pub data_dir: PathBuf,
    pub device_name: String,
    /// The relay new workspaces sync through (`LT_RELAY_URL`); none keeps
    /// them on this device.
    pub relay_url: Option<String>,
    /// The longest wait between sync rounds.
    pub sync_every: Duration,
    /// Opens the passkey hand-off page.
    pub opener: Arc<dyn Opener>,
    /// How long a passkey prompt waits for the browser.
    pub presence_wait: Duration,
    /// Answer with the Claude Code CLI when a chat or agent names no
    /// workspace runtime (the desktop's keyless fallback). Off in tests.
    pub cli_fallback: bool,
}

impl RealConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        let host = hive_runtime::machine_hostname().unwrap_or_else(|| "this machine".into());
        Self {
            data_dir,
            device_name: format!("lt on {host}"),
            relay_url: None,
            sync_every: Duration::from_secs(3),
            opener: Arc::new(presence::SystemOpener),
            presence_wait: presence::WAIT,
            cli_fallback: true,
        }
    }
}

/// The real backend. Cheap to clone.
#[derive(Clone)]
pub struct RealBackend {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    cfg: RealConfig,
    tx: broadcast::Sender<Event>,
    local: Mutex<LocalState>,
    list: Mutex<WorkspaceList>,
    core: RwLock<Option<Arc<Core>>>,
    presence: Arc<TermPresence>,
    secrets: Arc<dyn SecretStore>,
    /// Per workspace, the stop switch of its sync and host loops.
    loops: Mutex<HashMap<Uuid, tokio::sync::watch::Sender<bool>>>,
    snaps: Mutex<watch::Snaps>,
    /// Per streaming message, how much of its body the UI has been sent
    /// (by the turn runner as it streams, or by the translator from the log),
    /// so the two never send the same text twice.
    emitted: Mutex<HashMap<Uuid, usize>>,
}

/// The open device.
pub(crate) struct Core {
    identity: StoredIdentity,
    svc: Arc<Mutex<ChatService>>,
    reader: Mutex<ChatService>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// One line for the UI; a cancelled passkey prompt is just `cancelled`.
pub(crate) fn one_line(e: impl std::fmt::Display) -> BackendError {
    let s = e.to_string();
    let s = s.strip_prefix("unauthorized: ").unwrap_or(&s).to_string();
    if s.ends_with("cancelled") {
        return BackendError("cancelled".into());
    }
    BackendError(s.lines().next().unwrap_or_default().to_string())
}

fn ws_uuid(w: &WorkspaceId) -> Result<Uuid> {
    Uuid::parse_str(w.as_str()).map_err(|_| BackendError("no such workspace".into()))
}

fn uuid_of(s: &str, what: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|_| BackendError(format!("no such {what}")))
}

impl RealBackend {
    /// Open the device in `cfg.data_dir` (must run inside a tokio runtime).
    pub fn open(cfg: RealConfig) -> std::result::Result<Self, String> {
        // Create it owner-only first (keys, the database and the secrets live
        // here), then claim it as a Ligus data dir, as the CLI does.
        hive_runtime::private_file::ensure_private_dir(&cfg.data_dir).map_err(|e| format!("creating {}: {e}", cfg.data_dir.display()))?;
        hive_core::compat::claim_data_dir(&cfg.data_dir, hive_core::compat::home_dir().as_deref())?;
        hive_runtime::secret_store::set_binary_identifier(SIGNING_IDENTIFIER);
        let secrets = hive_runtime::secret_store::init_for_data_dir(&cfg.data_dir, None);
        let rt = tokio::runtime::Handle::try_current().map_err(|e| e.to_string())?;
        let (tx, _) = broadcast::channel(4096);
        let presence =
            Arc::new(TermPresence::new(tx.clone(), rt.clone(), cfg.data_dir.join("presence"), cfg.opener.clone(), cfg.presence_wait));
        let list = WorkspaceList::load(&cfg.data_dir, secrets.as_ref(), SECRET_SCOPE);
        let inner = Arc::new(Inner {
            local: Mutex::new(LocalState::load(&cfg.data_dir)),
            list: Mutex::new(list),
            core: RwLock::new(None),
            presence,
            secrets,
            loops: Mutex::new(HashMap::new()),
            snaps: Mutex::new(watch::Snaps::default()),
            emitted: Mutex::new(HashMap::new()),
            cfg,
            tx,
        });
        let identity = inner.identity_store().load().map_err(|e| e.to_string())?;
        if let Some(stored) = identity {
            inner.open_core(stored).map_err(|e| e.0)?;
            let ids: Vec<Uuid> = lock(&inner.list).workspaces.iter().map(|w| w.id()).collect();
            for ws in ids {
                inner.activate(ws);
                inner.start_loops(ws);
            }
        }
        Ok(Self { inner })
    }

    /// Bind the git checkout this device applies proposals in, for
    /// `workspace` (the apply-on-host loop uses its per-chat worktrees).
    pub fn bind_checkout(&self, workspace: &WorkspaceId, repo: &Path) -> Result<()> {
        let mut l = lock(&self.inner.local);
        l.checkouts.insert(workspace.0.clone(), repo.display().to_string());
        l.save(&self.inner.cfg.data_dir).map_err(one_line)
    }

    /// The write service (integration tests seed runtimes and agents).
    pub fn service(&self) -> Option<Arc<Mutex<ChatService>>> {
        self.inner.core().ok().map(|c| c.svc.clone())
    }

    /// This device's id, once it has an identity.
    pub fn device_id(&self) -> Option<Uuid> {
        self.inner.core().ok().map(|c| c.identity.device.id)
    }

    /// Stop the background loops (sync, hosts, apply).
    pub fn shutdown(&self) {
        for (_, stop) in lock(&self.inner.loops).drain() {
            let _ = stop.send(true);
        }
    }

    async fn call<T: Send + 'static>(&self, f: impl FnOnce(&Arc<Inner>) -> Result<T> + Send + 'static) -> Result<T> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || f(&inner)).await.map_err(one_line)?
    }
}

impl Inner {
    fn identity_store(&self) -> IdentityStore<StoreKeyVault> {
        IdentityStore::new(&self.cfg.data_dir, StoreKeyVault::for_data_dir(&self.cfg.data_dir))
    }

    fn db(&self) -> PathBuf {
        self.cfg.data_dir.join(DB_FILE)
    }

    pub(crate) fn emit(&self, e: Event) {
        let _ = self.tx.send(e);
    }

    pub(crate) fn core(&self) -> Result<Arc<Core>> {
        self.core.read().unwrap_or_else(|e| e.into_inner()).clone().ok_or_else(|| BackendError("create or join a workspace first".into()))
    }

    fn service_for(&self, stored: &StoredIdentity) -> Result<ChatService> {
        let ids = self.identity_store();
        let device_kp = ids.device_keypair(stored.device.id).map_err(one_line)?;
        let account_kp = ids.account_keypair(stored.account.id).map_err(one_line)?;
        let store = EventStore::open(self.db()).map_err(one_line)?;
        Ok(ChatService::new(store, stored.device.id, device_kp, account_kp, stored.actor()))
    }

    /// Open the services for `stored` (idempotent).
    fn open_core(self: &Arc<Self>, stored: StoredIdentity) -> Result<Arc<Core>> {
        if let Ok(c) = self.core() {
            return Ok(c);
        }
        let mut svc = self.service_for(&stored)?;
        svc.set_agent_vault(Box::new(StoreKeyVault::for_data_dir(&self.cfg.data_dir)));
        svc.set_presence_provider(Some(Arc::new(Provider(self.presence.clone()))));
        let reader = self.service_for(&stored)?;
        let feed = svc.change_feed();
        let core = Arc::new(Core { identity: stored, svc: Arc::new(Mutex::new(svc)), reader: Mutex::new(reader) });
        *self.core.write().unwrap_or_else(|e| e.into_inner()) = Some(core.clone());
        watch::prime(self, &core);
        watch::spawn_translator(self.clone(), feed.subscribe());
        Ok(core)
    }

    /// The identity, created with `your_name` on first use.
    fn ensure_core(self: &Arc<Self>, your_name: &str) -> Result<Arc<Core>> {
        if let Ok(c) = self.core() {
            return Ok(c);
        }
        let name = your_name.trim();
        if name.is_empty() {
            return err("tell us your name first");
        }
        let handle: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' }).collect();
        let stored = self.identity_store().bootstrap(name, &handle, &self.cfg.device_name).map_err(one_line)?;
        self.open_core(stored)
    }

    fn saved(&self, ws: Uuid) -> Result<SavedWorkspace> {
        lock(&self.list).get(ws).cloned().ok_or_else(|| BackendError("no such workspace".into()))
    }

    fn founder(&self, ws: Uuid) -> Option<String> {
        lock(&self.list).get(ws).and_then(|w| w.founder_actor_id.clone())
    }

    fn save_list(&self) -> Result<()> {
        lock(&self.list).save(&self.cfg.data_dir, self.secrets.as_ref(), SECRET_SCOPE).map_err(one_line)
    }

    fn save_local(&self) {
        let _ = lock(&self.local).save(&self.cfg.data_dir);
    }

    /// Run `f` on the write service, pinned to `ws`'s founder.
    pub(crate) fn write<T>(&self, ws: Uuid, f: impl FnOnce(&mut ChatService) -> std::result::Result<T, ChatError>) -> Result<T> {
        let core = self.core()?;
        let mut svc = lock(&core.svc);
        svc.set_trusted_founder(self.founder(ws));
        f(&mut svc).map_err(one_line)
    }

    /// Run `f` on the read service, pinned to `ws`'s founder. Never waits on a
    /// write (or a passkey prompt).
    pub(crate) fn read<T>(&self, ws: Uuid, f: impl FnOnce(&ChatService) -> std::result::Result<T, ChatError>) -> Result<T> {
        let core = self.core()?;
        let mut r = lock(&core.reader);
        r.set_trusted_founder(self.founder(ws));
        f(&r).map_err(one_line)
    }

    fn me(&self) -> String {
        self.core().map(|c| c.identity.actor().id).unwrap_or_default()
    }

    /// Make this device present in `ws`: the founder pin, its membership (a
    /// joiner redeems its invite once the config log has synced), its
    /// key-agreement key, and itself as a host. Best effort; runs again
    /// after each join and at startup.
    fn activate(&self, ws: Uuid) {
        let Ok(saved) = self.saved(ws) else { return };
        let Ok(core) = self.core() else { return };
        let ka = self.identity_store().key_agreement(core.identity.device.id).ok();
        let device = core.identity.device.id.to_string();
        let label = self.cfg.device_name.clone();
        let _ = self.write(ws, |svc| {
            if let Some((f, fk)) = saved.founder_actor_id.as_deref().zip(saved.fk.as_deref()) {
                let _ = svc.store().set_workspace_pin(ws, f, fk);
            }
            let config = workspace_config_session_id(ws);
            if svc.load(config)?.is_none() && svc.store().join_invite(ws)?.is_some() {
                return Ok(());
            }
            svc.ensure_self_member(config, ws)?;
            if let Some(ka) = &ka {
                let _ = svc.publish_key_agreement(ws, ka.public_key_bytes());
            }
            let _ = svc.ensure_host(ws, hive_core::WorkspaceHost::new(device, hive_core::HostKind::Device, label));
            Ok(())
        });
    }

    fn summary(&self, w: &SavedWorkspace) -> WorkspaceSummary {
        let ws = w.id();
        let me = self.me();
        let role = self
            .read(ws, |svc| Ok(svc.load(workspace_config_session_id(ws))?.map(|c| c.role_of(&me))))
            .ok()
            .flatten()
            .map(views::role)
            .unwrap_or(Role::Viewer);
        let can_invite = if w.relay().is_none() {
            Allowed::No("this workspace has no relay, so an invite to it can't be used (start lt with LT_RELAY_URL)".into())
        } else if role >= Role::Admin {
            Allowed::Yes
        } else {
            Allowed::No("only an Admin or Owner issues invites".into())
        };
        let id = ws.to_string();
        let mentioned = lock(&self.local).notifications.iter().any(|n| !n.read && n.workspace == id);
        WorkspaceSummary {
            id: WorkspaceId::new(id),
            name: w.name.clone(),
            mentioned,
            sync: w.relay().map(|r| SyncMode::Relay(r.to_string())).unwrap_or(SyncMode::Local),
            your_role: role,
            can_invite,
        }
    }

    fn workspaces(&self) -> Vec<WorkspaceSummary> {
        let list = lock(&self.list).workspaces.clone();
        list.iter().map(|w| self.summary(w)).collect()
    }

    /// The key source this device syncs `w` with (B1).
    fn key_source(&self, w: &SavedWorkspace) -> Result<hive_runtime::keyring::KeySource> {
        let core = self.core()?;
        let device = core.identity.device.id;
        let ka = self.identity_store().key_agreement(device).map_err(one_line)?;
        Ok(hive_runtime::keyring::KeySource::for_device(device, ka, w.secrets.key.as_deref(), &w.secrets.ek))
    }

    fn sync_engine(&self, w: &SavedWorkspace) -> Result<hive_runtime::SyncEngine> {
        let relay = w.relay().ok_or_else(|| BackendError("this workspace has no relay".into()))?;
        let client = hive_runtime::RelayClient::new(relay).with_auth(w.secrets.token.clone());
        Ok(hive_runtime::SyncEngine::new(client, w.room.clone()).with_key_source(self.key_source(w)?))
    }

    /// Whether chats in `ws` can only ever be answered here (no relay).
    pub(crate) fn local_only(&self, ws: Uuid) -> bool {
        lock(&self.list).get(ws).is_none_or(|w| w.relay().is_none())
    }

    /// The passkey for a new workspace: a browser `create` ceremony.
    fn create_passkey(&self, core: &Core, your_name: &str, ws_name: &str) -> Result<PasskeyCredential> {
        let account = core.identity.account.id;
        let mut challenge = [0u8; 32];
        challenge[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        challenge[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        let policy = PresencePolicy::ligus();
        let req = PresenceRequest {
            challenge,
            kind: "passkeyRegistered".into(),
            account_id: account,
            credential_ids: Vec::new(),
            digest_short: hive_core::keys::hex(&challenge[..8]),
            summary: format!("Create a Ligus passkey for “{ws_name}”"),
            policy: policy.clone(),
        };
        let att = self.presence.create(&req, your_name).map_err(one_line)?;
        let (id, alg, pk) = policy.with(|p| hive_core::presence::verify_create(&att, &challenge, p)).map_err(|e| BackendError(e.0))?;
        Ok(PasskeyCredential {
            account_id: account,
            credential_id: b64url(&id),
            alg,
            public_key: pk,
            label: format!("lt: {}", self.cfg.device_name).chars().take(64).collect(),
            recovery: false,
            registered_at: Timestamp::now(),
        })
    }

    fn invite(&self, ws: Uuid) -> Result<String> {
        let w = self.saved(ws)?;
        if let Some(problem) = invite_relay_problem(&w.relay_url) {
            return Err(BackendError(problem));
        }
        let core = self.core()?;
        let src = self.key_source(&w)?;
        let mut conn: WorkspaceConn = w.conn();
        let cert = core.identity.device.certificate.clone();
        self.write(ws, |svc| {
            let inv = svc.issue_invite(ws)?;
            conn.inv = Some(inv.to_string());
            let ek = svc.delivered_epoch_keys(ws, &src)?;
            if !ek.is_empty() {
                conn.ek = ek;
            }
            let (name, ek_json) = (conn.display_name(), conn.ek_json());
            conn.sig = svc.sign_invite(&conn.sig_claims(&name, &ek_json), &cert);
            Ok(())
        })?;
        Ok(encode_workspace_invite(&conn))
    }
}

#[async_trait]
impl Backend for RealBackend {
    fn device(&self) -> DeviceInfo {
        let core = self.inner.core().ok();
        let safety_code = core
            .as_ref()
            .and_then(|c| {
                let fp = hive_runtime::envelope_verifier::FounderPin::fingerprint_hex(&c.identity.account.signing_public_key);
                hive_runtime::directory_trust::safety_code(&fp)
            })
            .unwrap_or_default();
        DeviceInfo {
            device_name: self.inner.cfg.device_name.clone(),
            safety_code,
            data_dir: self.inner.cfg.data_dir.display().to_string(),
            secret_store: self.inner.secrets.backend().as_str().to_string(),
            backend_label: "ligus".into(),
        }
    }

    fn events(&self) -> BoxStream<'static, Event> {
        let rx = self.inner.tx.subscribe();
        Box::pin(futures::stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok(e) => Some((e, rx)),
                // A slow reader reloads what it shows.
                Err(broadcast::error::RecvError::Lagged(_)) => Some((Event::WorkspacesChanged, rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        }))
    }

    async fn startup(&self) -> Result<StartupState> {
        self.call(|i| {
            let your_name = i.core().ok().map(|c| c.identity.account.display_name.clone());
            Ok(StartupState { your_name, workspaces: i.workspaces() })
        })
        .await
    }

    async fn create_workspace(&self, your_name: &str, name: &str) -> Result<Created> {
        let (your_name, name) = (your_name.trim().to_string(), name.trim().to_string());
        if name.is_empty() {
            return err("give the workspace a name");
        }
        let created = self
            .call(move |i| {
                let core = i.ensure_core(&your_name)?;
                let passkey = i.create_passkey(&core, &your_name, &name)?;
                let account = core.identity.account.id;
                let fk = hive_runtime::envelope_verifier::FounderPin::fingerprint_hex(&core.identity.account.signing_public_key);
                let conn = WorkspaceConn {
                    name: name.clone(),
                    relay_url: i.cfg.relay_url.clone().unwrap_or_default(),
                    room: random_token(32),
                    key: Some(random_token(24)),
                    icon: None,
                    dm_account: None,
                    founder_actor_id: Some(core.identity.actor().id),
                    fk: Some(fk),
                    inv: None,
                    ek: Vec::new(),
                    sig: None,
                };
                let ws = conn.id();
                lock(&i.list).upsert(SavedWorkspace::from_conn(&conn, None));
                i.save_list()?;
                let key = RecoveryKey::generate().map_err(|e| BackendError(e.0))?;
                i.write(ws, |svc| {
                    let config = svc.ensure_workspace_config(ws)?;
                    svc.ensure_self_member(config, ws)?;
                    // The recovery key proves itself; it then confirms the
                    // passkey's registration, so one browser ceremony does.
                    svc.register_passkey(ws, key.credential(account, "Recovery key"), &|req| Ok(key.assert(&req.challenge)))?;
                    svc.register_passkey(ws, passkey, &|req| Ok(key.assert(&req.challenge)))?;
                    Ok(())
                })?;
                i.activate(ws);
                let summary = i.summary(&i.saved(ws)?);
                Ok((ws, summary, key.words()))
            })
            .await?;
        let (ws, summary, words) = created;
        self.inner.start_loops(ws);
        self.inner.emit(Event::WorkspacesChanged);
        Ok(Created { workspace: summary, recovery_words: words.split_whitespace().map(str::to_string).collect(), invite: None })
    }

    async fn join_workspace(&self, your_name: &str, invite: &str) -> Result<WorkspaceSummary> {
        let mut conn = decode_workspace_invite(invite).map_err(BackendError)?;
        if conn.dm_account.is_some() {
            return err("that's a direct-message invite; lt doesn't open direct messages yet");
        }
        if let Some(problem) = invite_relay_problem(&conn.relay_url) {
            return Err(BackendError(problem));
        }
        let ws = conn.id();
        let your_name = your_name.trim().to_string();
        let saved = self
            .call(move |i| {
                i.ensure_core(&your_name)?;
                let store = EventStore::open(i.db()).map_err(one_line)?;
                if let Some(inv) = conn.invite_id() {
                    store.set_join_invite(ws, inv).map_err(one_line)?;
                }
                if let Some((f, fk)) = conn.founder_pin() {
                    store.set_workspace_pin(ws, &f, &fk).map_err(one_line)?;
                }
                conn.sig = None;
                let saved = SavedWorkspace::from_conn(&conn, None);
                lock(&i.list).upsert(saved.clone());
                i.save_list()?;
                Ok(saved)
            })
            .await?;
        // Pull the workspace (its config log) before redeeming the invite.
        let mut engine = self
            .call({
                let saved = saved.clone();
                move |i| i.sync_engine(&saved)
            })
            .await?;
        let mut store = EventStore::open(self.inner.db()).map_err(one_line)?;
        engine.catch_up(&mut store, 250).await.map_err(one_line)?;
        self.call(move |i| {
            i.activate(ws);
            let me = i.me();
            let joined =
                i.read(ws, |svc| Ok(svc.load(workspace_config_session_id(ws))?.is_some_and(|c| c.members.iter().any(|m| m.id == me))))?;
            if !joined {
                return err("couldn't join: the workspace didn't accept the invite (it may be used up, expired or revoked)");
            }
            Ok(())
        })
        .await?;
        hive_runtime::sync_service::sync_round(&mut engine, &mut store).await.map_err(one_line)?;
        self.inner.start_loops(ws);
        self.inner.emit(Event::WorkspacesChanged);
        self.call(move |i| Ok(i.summary(&i.saved(ws)?))).await
    }

    async fn confirm_recovery_saved(&self, workspace: &WorkspaceId) -> Result<()> {
        let id = workspace.0.clone();
        self.call(move |i| {
            lock(&i.local).recovery_saved.insert(id);
            i.save_local();
            Ok(())
        })
        .await
    }

    async fn workspaces(&self) -> Result<Vec<WorkspaceSummary>> {
        self.call(|i| Ok(i.workspaces())).await
    }

    async fn create_invite(&self, workspace: &WorkspaceId, role: Role) -> Result<String> {
        if role != Role::Contributor {
            return err("an invite admits a Contributor; change their role once they've joined");
        }
        let ws = ws_uuid(workspace)?;
        self.call(move |i| i.invite(ws)).await
    }

    async fn chats(&self, workspace: &WorkspaceId) -> Result<Vec<ChatSummary>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| i.chats(ws)).await
    }

    async fn create_chat(&self, workspace: &WorkspaceId, title: &str) -> Result<ChatSummary> {
        let ws = ws_uuid(workspace)?;
        let title = match title.trim() {
            "" => "New chat".to_string(),
            t => t.to_string(),
        };
        self.call(move |i| {
            let runtime = i.read(ws, |svc| svc.list_workspace_runtimes(ws))?.first().map(|r| r.id.clone()).unwrap_or_default();
            let s = i.write(ws, |svc| svc.create_chat(title, ws, runtime))?;
            i.chats(ws)?.into_iter().find(|c| c.id.0 == s.id.to_string()).ok_or_else(|| BackendError("the chat vanished".into()))
        })
        .await
    }

    async fn transcript(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<Vec<Message>> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        self.call(move |i| {
            let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
            Ok(views::transcript(&s))
        })
        .await
    }

    async fn send_message(&self, workspace: &WorkspaceId, chat: &ChatId, text: &str) -> Result<Message> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        let text = text.trim_end().to_string();
        if text.trim().is_empty() {
            return err("nothing to send");
        }
        let (message, plan) = self.call(move |i| turns::post_and_plan(i, ws, sid, &text)).await?;
        if let Some(plan) = plan {
            turns::spawn_turn(self.inner.clone(), plan);
        }
        Ok(message)
    }

    async fn mentionables(&self, workspace: &WorkspaceId) -> Result<Vec<Mentionable>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let me = i.me();
            let config = i.read(ws, |svc| svc.load(workspace_config_session_id(ws)))?;
            let mut out = vec![
                Mentionable { handle: "primary".into(), kind: "Default agent".into() },
                Mentionable { handle: "all".into(), kind: "Everyone".into() },
            ];
            if let Some(c) = config {
                out.extend(c.workspace_agents.iter().map(|a| Mentionable { handle: a.name.clone(), kind: "Agent".into() }));
                out.extend(
                    c.members
                        .iter()
                        .filter(|m| m.id != me)
                        .map(|m| Mentionable { handle: m.actor.display_name.clone(), kind: views::role(m.role).label().to_string() }),
                );
            }
            Ok(out)
        })
        .await
    }

    async fn mark_read(&self, _workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        let chat = chat.0.clone();
        self.call(move |i| {
            {
                let mut l = lock(&i.local);
                l.read.insert(chat.clone(), views::ms(&Timestamp::now()));
                for n in l.notifications.iter_mut().filter(|n| n.chat.as_deref() == Some(chat.as_str())) {
                    n.read = true;
                }
            }
            i.save_local();
            Ok(())
        })
        .await
    }

    async fn lock_chat(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        self.call(move |i| {
            i.write(ws, |svc| svc.self_medicate(ws, sid, None, &[(hive_core::guard::LockdownSignal::Manual, "manual".into())]))?;
            Ok(())
        })
        .await
    }

    async fn clear_lockdown(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        self.call(move |i| {
            let me = i.me();
            let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
            let view = hive_runtime::lockdown_view::chat_lockdown(&s, sid, &me);
            if !view.can_clear {
                return err("only a human Contributor or above clears a lockdown");
            }
            i.write(ws, |svc| svc.clear_lockdown(ws, sid, view.raise_ids()))
        })
        .await
    }

    async fn proposals(&self, workspace: &WorkspaceId) -> Result<Vec<Proposal>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| i.proposals(ws)).await
    }

    async fn proposal_diff(&self, workspace: &WorkspaceId, proposal: &ProposalId) -> Result<Diff> {
        let (ws, pid) = (ws_uuid(workspace)?, uuid_of(proposal.as_str(), "proposal")?);
        self.call(move |i| {
            let (_, p) = i.find_proposal(ws, pid)?;
            Ok(views::parse_diff(p.diff.as_deref().unwrap_or_default()))
        })
        .await
    }

    async fn vote(&self, workspace: &WorkspaceId, proposal: &ProposalId, choice: VoteChoice) -> Result<Proposal> {
        let (ws, pid) = (ws_uuid(workspace)?, uuid_of(proposal.as_str(), "proposal")?);
        self.call(move |i| {
            let (sid, _) = i.find_proposal(ws, pid)?;
            i.write(ws, |svc| svc.vote_on_proposal(sid, ws, pid, choice == VoteChoice::Approve))?;
            i.proposal_view(ws, sid, pid)
        })
        .await
    }

    async fn request_apply(&self, workspace: &WorkspaceId, proposal: &ProposalId, host: &HostId) -> Result<Proposal> {
        let (ws, pid) = (ws_uuid(workspace)?, uuid_of(proposal.as_str(), "proposal")?);
        let host = uuid_of(host.as_str(), "host")?;
        self.call(move |i| {
            let (sid, _) = i.find_proposal(ws, pid)?;
            i.write(ws, |svc| svc.request_apply(sid, ws, pid, host, None))?;
            i.proposal_view(ws, sid, pid)
        })
        .await
    }

    async fn withdraw_apply(&self, workspace: &WorkspaceId, proposal: &ProposalId) -> Result<Proposal> {
        let (ws, pid) = (ws_uuid(workspace)?, uuid_of(proposal.as_str(), "proposal")?);
        self.call(move |i| {
            let (sid, _) = i.find_proposal(ws, pid)?;
            let s = i.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
            let request = s.open_apply_request(pid).map(|r| r.id).ok_or_else(|| BackendError("nothing to withdraw".into()))?;
            i.write(ws, |svc| svc.withdraw_apply_request(sid, ws, request))?;
            i.proposal_view(ws, sid, pid)
        })
        .await
    }

    async fn agents(&self, workspace: &WorkspaceId) -> Result<Vec<Agent>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let device = i.core()?.identity.device.id.to_string();
            let config = i.read(ws, |svc| svc.load(workspace_config_session_id(ws)))?;
            Ok(config
                .map(|c| c.workspace_agents)
                .unwrap_or_default()
                .into_iter()
                .map(|a| Agent {
                    id: AgentId::new(a.id.to_string()),
                    handle: a.name.clone(),
                    runtime: RuntimeId::new(a.runtime_id.clone()),
                    role: Some(a.role.clone()).filter(|r| !r.is_empty()),
                    host: Some(a.host_id.clone()).filter(|h| !h.is_empty() && *h != device).map(HostId::new),
                    shared: a.owner_actor_id.is_empty(),
                })
                .collect())
        })
        .await
    }

    async fn runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<Runtime>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let rts = i.read(ws, |svc| svc.list_workspace_runtimes(ws))?;
            Ok(rts
                .into_iter()
                .enumerate()
                .map(|(n, r)| Runtime {
                    id: RuntimeId::new(r.id.clone()),
                    label: r.name.clone(),
                    kind: views::runtime_kind(r.provider),
                    model: Some(r.model.clone()).filter(|m| !m.is_empty()),
                    configured: turns::configured(&r),
                    active: n == 0,
                })
                .collect())
        })
        .await
    }

    async fn hosts(&self, workspace: &WorkspaceId) -> Result<Vec<Host>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let device = i.core()?.identity.device.id.to_string();
            let hosts = i.read(ws, |svc| svc.list_hosts(ws))?;
            let presence = hive_runtime::HostPresence::new();
            let now = Timestamp::now();
            Ok(hosts
                .iter()
                .map(|h| Host {
                    id: HostId::new(h.id.clone()),
                    label: if h.id == device { "This device".into() } else { h.label.clone() },
                    kind: match h.kind {
                        hive_core::HostKind::Device => HostKind::Device,
                        hive_core::HostKind::Worker => HostKind::Worker,
                    },
                    online: h.id == device || presence.is_online(h, &now),
                    last_seen: Some(views::ms(&presence.last_seen(h))),
                    this_device: h.id == device,
                })
                .collect())
        })
        .await
    }

    async fn set_agent_runtime(&self, workspace: &WorkspaceId, agent: &AgentId, runtime: &RuntimeId) -> Result<()> {
        let (ws, aid) = (ws_uuid(workspace)?, uuid_of(agent.as_str(), "agent")?);
        let runtime = runtime.0.clone();
        self.call(move |i| {
            let config = workspace_config_session_id(ws);
            let mut a = i
                .read(ws, |svc| svc.load(config))?
                .and_then(|c| c.workspace_agents.into_iter().find(|a| a.id == aid))
                .ok_or_else(|| BackendError("no such agent".into()))?;
            a.runtime_id = runtime;
            i.write(ws, |svc| svc.add_agent(config, ws, a))
        })
        .await
    }

    async fn members(&self, workspace: &WorkspaceId) -> Result<Vec<Member>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let me = i.me();
            let founder = i.founder(ws);
            let (config, timeline) = i.read(ws, |svc| Ok(svc.store().load_config(ws, founder.as_deref(), None)?))?;
            let Some(config) = config else { return Ok(Vec::new()) };
            Ok(views::members(&config, timeline.current(), &me, &|_| false))
        })
        .await
    }

    async fn set_role(&self, workspace: &WorkspaceId, member: &MemberId, role: Role) -> Result<()> {
        let ws = ws_uuid(workspace)?;
        let member = member.0.clone();
        self.call(move |i| i.write(ws, |svc| svc.set_member_role(workspace_config_session_id(ws), ws, member, views::workspace_role(role))))
            .await
    }

    async fn remove_member(&self, workspace: &WorkspaceId, member: &MemberId) -> Result<()> {
        let ws = ws_uuid(workspace)?;
        let member = member.0.clone();
        self.call(move |i| i.write(ws, |svc| svc.remove_member(workspace_config_session_id(ws), ws, member))).await
    }

    async fn notifications(&self) -> Result<Vec<Notification>> {
        self.call(|i| {
            Ok(lock(&i.local)
                .notifications
                .iter()
                .map(|n| Notification {
                    id: NotificationId::new(n.id.clone()),
                    workspace: WorkspaceId::new(n.workspace.clone()),
                    chat: n.chat.clone().map(ChatId::new),
                    text: n.text.clone(),
                    at: n.at,
                    read: n.read,
                })
                .collect())
        })
        .await
    }

    async fn mark_notifications_read(&self) -> Result<()> {
        self.call(|i| {
            for n in lock(&i.local).notifications.iter_mut() {
                n.read = true;
            }
            i.save_local();
            Ok(())
        })
        .await
    }

    async fn settings(&self) -> Result<Settings> {
        self.call(|i| Ok(i.settings())).await
    }

    async fn set_setting(&self, key: SettingKey, on: bool) -> Result<Settings> {
        self.call(move |i| {
            {
                let mut l = lock(&i.local);
                match key {
                    SettingKey::NotifyMentions => l.notify_mentions = on,
                    SettingKey::AgentsMayEdit => l.agents_may_edit = on,
                }
            }
            i.save_local();
            Ok(i.settings())
        })
        .await
    }

    async fn answer_presence(&self, id: PresenceId, answer: PresenceAnswer) -> Result<()> {
        self.inner.presence.answer(id, answer).map_err(BackendError)
    }
}

impl Inner {
    fn settings(&self) -> Settings {
        let l = lock(&self.local);
        Settings {
            your_name: self.core().map(|c| c.identity.account.display_name.clone()).unwrap_or_default(),
            device_name: self.cfg.device_name.clone(),
            notify_mentions: l.notify_mentions,
            agents_may_edit: l.agents_may_edit,
            relay_url: self.cfg.relay_url.clone(),
        }
    }

    /// The chat sessions of `ws` (never its config log).
    pub(crate) fn chat_sessions(&self, ws: Uuid) -> Result<Vec<hive_core::ChatSession>> {
        let config = workspace_config_session_id(ws);
        self.read(ws, |svc| {
            let mut out = Vec::new();
            for sid in svc.store().sessions_of_workspace(ws)? {
                if sid == config {
                    continue;
                }
                if let Some(s) = svc.load(sid)?
                    && !s.archived
                {
                    out.push(s);
                }
            }
            Ok(out)
        })
    }

    fn chats(&self, ws: Uuid) -> Result<Vec<ChatSummary>> {
        let me = self.me();
        let channels = self.read(ws, |svc| svc.list_channels(ws)).unwrap_or_default();
        let (read, notes) = {
            let l = lock(&self.local);
            (l.read.clone(), l.notifications.clone())
        };
        let mut out: Vec<ChatSummary> = self
            .chat_sessions(ws)?
            .iter()
            .map(|s| {
                let id = s.id.to_string();
                let last = s.messages.iter().rev().find(|m| m.role != hive_core::MessageRole::System);
                let updated_at = last.map(|m| views::ms(&m.created_at)).unwrap_or_else(|| views::ms(&s.created_at));
                let seen = read.get(&id).copied().unwrap_or(0);
                let unread = last.is_some_and(|m| views::ms(&m.created_at) > seen && m.actor_identity.as_ref().is_none_or(|a| a.id != me));
                let view = hive_runtime::lockdown_view::chat_lockdown(s, s.id, &me);
                ChatSummary {
                    id: ChatId::new(id.clone()),
                    title: s.title.clone(),
                    channel: channels.iter().find(|c| c.id == s.channel_id).map(|c| c.name.clone()),
                    updated_at,
                    unread,
                    mentioned: notes.iter().any(|n| !n.read && n.chat.as_deref() == Some(id.as_str())),
                    lockdown: view.active().then(|| Lockdown {
                        reasons: view.signals.iter().map(|l| format!("{:?} {}", l.signal, l.detail).trim().to_string()).collect(),
                        clear: if view.can_clear {
                            Allowed::Yes
                        } else {
                            Allowed::No("only a human Contributor or above clears a lockdown".into())
                        },
                    }),
                }
            })
            .collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(out)
    }

    fn host_labels(&self, ws: Uuid) -> Vec<(String, String)> {
        let device = self.core().map(|c| c.identity.device.id.to_string()).unwrap_or_default();
        self.read(ws, |svc| svc.list_hosts(ws))
            .unwrap_or_default()
            .into_iter()
            .map(|h| {
                let label = if h.id == device { "This device".to_string() } else { h.label };
                (h.id, label)
            })
            .collect()
    }

    pub(crate) fn proposals(&self, ws: Uuid) -> Result<Vec<Proposal>> {
        let me = self.me();
        let hosts = self.host_labels(ws);
        let mut out: Vec<Proposal> = self
            .chat_sessions(ws)?
            .iter()
            .flat_map(|s| s.proposals.iter().filter(|p| !p.dismissed).map(|p| views::proposal(s, p, &me, &hosts)).collect::<Vec<_>>())
            .collect();
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    fn find_proposal(&self, ws: Uuid, pid: Uuid) -> Result<(Uuid, hive_core::ActionProposal)> {
        self.chat_sessions(ws)?
            .into_iter()
            .find_map(|s| s.proposals.iter().find(|p| p.id == pid).cloned().map(|p| (s.id, p)))
            .ok_or_else(|| BackendError("no such proposal".into()))
    }

    pub(crate) fn proposal_view(&self, ws: Uuid, sid: Uuid, pid: Uuid) -> Result<Proposal> {
        let me = self.me();
        let hosts = self.host_labels(ws);
        let s = self.read(ws, |svc| svc.load(sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
        let p = s.proposals.iter().find(|p| p.id == pid).ok_or_else(|| BackendError("no such proposal".into()))?;
        Ok(views::proposal(&s, p, &me, &hosts))
    }
}
