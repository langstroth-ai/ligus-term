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
//! (`ligus_runtime::turn_dispatch`).

mod deploy;
pub(crate) mod detect;
mod link;
mod local;
mod mcp;
mod scratch;
mod setup_ops;
mod term_presence;
mod turns;
pub mod views;
mod watch;
mod workflows;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use ligus_core::presence::{PasskeyCredential, PresencePolicy, RecoveryKey, b64url};
use ligus_core::{Timestamp, workspace_config_session_id};
use ligus_runtime::chat_service::{ChatError, PresenceRequest};
use ligus_runtime::connection_settings::{SavedWorkspace, WorkspaceList};
use ligus_runtime::secret_store::{SecretStore, StoreKeyVault};
use ligus_runtime::workspace_invite::{
    WorkspaceConn, decode_workspace_invite, encode_workspace_invite, invite_relay_problem, random_token,
};
use ligus_runtime::{ChatService, EventStore, IdentityStore, StoredIdentity};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::model::*;
use super::presence::{self, Opener};
use super::{Backend, BackendError, Event, Result, err};
use local::LocalState;
pub use local::SECRET_SCOPE;
use term_presence::{Provider, TermPresence};
pub use turns::{DeferredClock, DeferredHand};

/// The database file under the data dir (the CLI's name).
pub const DB_FILE: &str = "ligus.db";
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
    /// The loopback port passkey prompts listen on (`LT_PRESENCE_PORT`; 0:
    /// any free port).
    pub presence_port: u16,
    /// Show the passkey page's address instead of opening a browser
    /// (`LT_NO_BROWSER`), for a browser on another device.
    pub no_browser: bool,
    /// Answer with the Claude Code CLI when a chat or agent names no
    /// workspace runtime (the desktop's keyless fallback). Off in tests.
    pub cli_fallback: bool,
    /// How often this device publishes its host heartbeat (and the leases
    /// of the runs it drives) and reads everyone else's.
    pub beat_every: Duration,
    /// How long a workflow stage waits for its agent's host to reply.
    pub stage_timeout: Duration,
    /// How long a turn deferred to its owner's device waits before this
    /// device may answer it (the deferred-claim fallback).
    pub deferred_timeout: Duration,
    /// Tests: a hand-driven clock the deferred-claim fallback waits on
    /// instead of `deferred_timeout`.
    pub deferred_clock: Option<DeferredClock>,
    /// What runtime detection reads (`PATH`, home, credential markers);
    /// `None` reads this process's ([`detect::process_env`]). Tests point it
    /// at a temporary `PATH` of stub CLIs.
    pub detect_env: Option<ligus_runtime::cli_detect::DetectEnv>,
    /// The Ollama server detection probes and device-local Ollama agents
    /// talk to; `None` skips it.
    pub ollama_url: Option<String>,
}

impl RealConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        let host = ligus_runtime::machine_hostname().unwrap_or_else(|| "this machine".into());
        Self {
            data_dir,
            device_name: format!("lt on {host}"),
            relay_url: None,
            sync_every: Duration::from_secs(3),
            opener: Arc::new(presence::SystemOpener),
            presence_wait: presence::WAIT,
            presence_port: 0,
            no_browser: false,
            cli_fallback: true,
            beat_every: Duration::from_secs(ligus_runtime::host_presence::HOST_BEAT_EVERY_SECS as u64),
            stage_timeout: Duration::from_secs(300),
            deferred_timeout: ligus_runtime::turn_dispatch::DEFERRED_CLAIM_TIMEOUT,
            deferred_clock: None,
            detect_env: None,
            ollama_url: Some(detect::OLLAMA_DEFAULT.into()),
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
    /// Host heartbeats read from the relay's presence channel.
    /// Host beats per workspace: a beat read from one workspace's room
    /// never makes another workspace's host look online.
    beats: Mutex<HashMap<Uuid, ligus_runtime::HostPresence>>,
    /// The workflow runs this device drives.
    drivers: Mutex<HashMap<Uuid, workflows::Driving>>,
    /// Per workspace, this device's Ligus MCP server for its agents' turns.
    ligus: Mutex<HashMap<Uuid, Arc<ligus_runtime::ligus_mcp::LigusMcp>>>,
    /// The turns running on this device, each with its own stop signal.
    turn_stops: Arc<ligus_runtime::turn_engine::TurnStops>,
    /// A link is being redeemed here: the data dir's identity is in flux,
    /// so [`Inner::follow_disk`] leaves it alone.
    linking: std::sync::atomic::AtomicUsize,
    /// One [`Inner::follow_disk`] at a time.
    following: Mutex<()>,
}

/// The open device.
pub(crate) struct Core {
    identity: StoredIdentity,
    /// Who this device authors as: its own identity, or, linked as a
    /// person's device ("Add to my devices"), that person.
    author: ligus_core::ActorIdentity,
    /// Linked: the account of the person this device acts for.
    person: Option<Uuid>,
    /// Replaced by [`Inner::follow_disk`]: whoever still holds it signs
    /// nothing more with it.
    retired: std::sync::atomic::AtomicBool,
    svc: Arc<Mutex<ChatService>>,
    reader: Mutex<ChatService>,
    /// The write service's change feed, kept here so nothing has to lock the
    /// service (held through a passkey prompt) to reach it.
    feed: ligus_runtime::chat_service::ChangeFeed,
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
        // A relay new workspaces can't use is a mistake to say now, not a
        // workspace that silently stays on this device.
        if let Some(r) = cfg.relay_url.as_deref()
            && !ligus_runtime::workspace_invite::usable_relay_url(r)
        {
            return Err(format!("LT_RELAY_URL {r:?} isn't a relay address: give an http(s) URL, e.g. https://relay.example"));
        }
        // Create it owner-only (keys, the database and the secrets live here).
        ligus_runtime::private_file::ensure_private_dir(&cfg.data_dir).map_err(|e| format!("creating {}: {e}", cfg.data_dir.display()))?;
        ligus_runtime::secret_store::set_binary_identifier(SIGNING_IDENTIFIER);
        // No agent child (the keyless Claude Code fallback, any CLI runtime)
        // may read this device's keys, database or secrets: deny the data
        // dir in the agent sandbox, as the desktop and the CLI do theirs.
        // Its scratch repositories (`<data dir>/scratch`) are checkouts
        // agents work in, like any other: readable, as the CLI's repos dir.
        ligus_runtime::executor::configure_host(&cfg.data_dir, &[], &[cfg.data_dir.join("scratch")]);
        let secrets = ligus_runtime::secret_store::init_for_data_dir(&cfg.data_dir, None);
        let rt = tokio::runtime::Handle::try_current().map_err(|e| e.to_string())?;
        let (tx, _) = broadcast::channel(4096);
        let presence = Arc::new(
            TermPresence::new(tx.clone(), rt.clone(), cfg.data_dir.join("presence"), cfg.opener.clone(), cfg.presence_wait)
                .hand_off_on(cfg.presence_port, cfg.no_browser),
        );
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
            beats: Mutex::new(HashMap::new()),
            drivers: Mutex::new(HashMap::new()),
            ligus: Mutex::new(HashMap::new()),
            turn_stops: ligus_runtime::turn_engine::TurnStops::new(),
            linking: std::sync::atomic::AtomicUsize::new(0),
            following: Mutex::new(()),
            cfg,
            tx,
        });
        let identity = inner.identity_store().load().map_err(|e| e.to_string())?;
        if let Some(stored) = identity {
            inner.open_core(stored).map_err(|e| e.0)?;
            let ids: Vec<Uuid> = lock(&inner.list).workspaces.iter().map(|w| w.id()).collect();
            for ws in ids {
                // Activation writes (it may wait on the write lock or a
                // passkey): never on the async thread `open` runs on.
                let i = inner.clone();
                rt.spawn_blocking(move || i.activate(ws));
                inner.start_loops(ws);
            }
        }
        Ok(Self { inner })
    }

    /// Pull a just-saved workspace, redeem the invite and start syncing it.
    async fn finish_join(&self, ws: Uuid, saved: SavedWorkspace) -> Result<WorkspaceSummary> {
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
            let joined = i.read(ws, |svc| {
                Ok(svc.load_in(ws, workspace_config_session_id(ws))?.is_some_and(|c| c.members.iter().any(|m| m.id == me)))
            })?;
            if !joined {
                return err("couldn't join: the workspace didn't accept the invite (it may be used up, expired or revoked)");
            }
            Ok(())
        })
        .await?;
        ligus_runtime::sync_service::sync_round(&mut engine, &mut store).await.map_err(one_line)?;
        self.inner.start_loops(ws);
        self.inner.emit(Event::WorkspacesChanged);
        self.call(move |i| Ok(i.summary(&i.saved(ws)?))).await
    }

    /// Push what this device holds for `ws` to the relay now. A code handed
    /// out (an invite, a box admission) is redeemable only once the relay has
    /// its issuance, and the sync loop's next round may come after the code
    /// is used, or never (plain mode exits once it prints the code).
    async fn publish_now(&self, ws: Uuid) -> Result<()> {
        let mut engine = self.call(move |i| i.sync_engine(&i.saved(ws)?)).await?;
        let failed = |e: ligus_runtime::SyncError| BackendError(format!("couldn't put it on the relay ({}); try again", one_line(e).0));
        let unpushed = engine.take_unpushed(&EventStore::open(self.inner.db()).map_err(one_line)?).map_err(failed)?;
        engine.push_envelopes(&unpushed).await.map_err(failed)?;
        // Record them as pushed, so the sync loop needn't send them again
        // when it starts afresh.
        engine.take_unpushed(&EventStore::open(self.inner.db()).map_err(one_line)?).map_err(failed)?;
        Ok(())
    }

    /// Bind the git checkout this device applies proposals in, for
    /// `workspace` (the apply-on-host loop uses its per-chat worktrees).
    pub fn bind_checkout(&self, workspace: &WorkspaceId, repo: &Path) -> Result<()> {
        self.inner.bind(workspace, Some(repo)).map(|_| ())
    }

    /// The write service (integration tests seed runtimes and agents).
    pub fn service(&self) -> Option<Arc<Mutex<ChatService>>> {
        self.inner.core().ok().map(|c| c.svc.clone())
    }

    /// This device's id, once it has an identity.
    pub fn device_id(&self) -> Option<Uuid> {
        self.inner.core().ok().map(|c| c.identity.device.id)
    }

    /// Stop the background loops (sync, hosts, apply) and every turn
    /// running here (a turn that starts after this is born stopped).
    pub fn shutdown(&self) {
        for (_, stop) in lock(&self.inner.loops).drain() {
            let _ = stop.send(true);
        }
        self.inner.turn_stops.close();
    }

    async fn call<T: Send + 'static>(&self, f: impl FnOnce(&Arc<Inner>) -> Result<T> + Send + 'static) -> Result<T> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            inner.follow_disk();
            f(&inner)
        })
        .await
        .map_err(one_line)?
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
        Ok(ChatService::new(store, device_kp, account_kp, stored.actor()))
    }

    /// Open the services for `stored` (idempotent).
    fn open_core(self: &Arc<Self>, stored: StoredIdentity) -> Result<Arc<Core>> {
        if let Ok(c) = self.core() {
            return Ok(c);
        }
        let mut svc = self.service_for(&stored)?;
        svc.set_agent_vault(Box::new(StoreKeyVault::for_data_dir(&self.cfg.data_dir)));
        svc.set_presence_provider(Some(Arc::new(Provider(self.presence.clone()))));
        let mut reader = self.service_for(&stored)?;
        // Linked as a person's device: act as them, without their account key.
        let state = link::link_state(&self.cfg.data_dir)?;
        let person = state.person.as_ref().and_then(|p| p.account_id);
        // The person as the workspace's roster has them now (their name,
        // avatar), not as the grant named them on the day it was redeemed:
        // the profile is theirs, edited on their desktop.
        let linked = state.person.clone().map(|p| {
            state
                .workspaces
                .iter()
                .find_map(|ws| {
                    reader.set_trusted_founder(self.founder(*ws));
                    let config = reader.load_in(*ws, workspace_config_session_id(*ws)).ok().flatten()?;
                    let m =
                        config.members.into_iter().find(|m| m.id == p.id && m.actor.id == p.id && m.actor.account_id == p.account_id)?;
                    Some(ligus_runtime::computer_link::device_local_cleared(&m.actor))
                })
                .unwrap_or(p)
        });
        let author = match linked {
            Some(person) => {
                svc.act_as_owner_device(person.clone());
                reader.act_as_owner_device(person);
                svc.author().clone()
            }
            None => stored.actor(),
        };
        let feed = svc.change_feed();
        let core = Arc::new(Core {
            identity: stored,
            author,
            person,
            retired: std::sync::atomic::AtomicBool::new(false),
            svc: Arc::new(Mutex::new(svc)),
            reader: Mutex::new(reader),
            feed: feed.clone(),
        });
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

    /// Run `f` on the write service, pinned to `ws`'s founder. Refused
    /// while the open identity isn't the data dir's (another lt linked it
    /// since): nothing is signed with keys the data dir has left behind.
    pub(crate) fn write<T>(&self, ws: Uuid, f: impl FnOnce(&mut ChatService) -> std::result::Result<T, ChatError>) -> Result<T> {
        let core = self.core()?;
        self.may_sign(&core, ws)?;
        let mut svc = lock(&core.svc);
        svc.set_trusted_founder(self.founder(ws));
        f(&mut svc).map_err(|e| link::explain(&svc, ws, one_line(e)))
    }

    /// Whether `core` may sign in `ws`: refused while it isn't the data
    /// dir's identity (another lt linked it since), and, linked, anywhere it
    /// wasn't linked into (its own account there is a phantom). Every write
    /// path checks it: [`Inner::write`], setup, the MCP host, apply.
    pub(crate) fn may_sign(&self, core: &Core, ws: Uuid) -> Result<()> {
        if core.retired.load(std::sync::atomic::Ordering::SeqCst) || self.stale(core) {
            return err(link::STALE);
        }
        // While a link is redeemed here, the device key switches before the
        // link is recorded: a write in between (an activation or heartbeat
        // still in flight) would sign with the new key as this install's own
        // member and certify it for the wrong account. Nothing but the
        // linked identity signs until the redeem is done.
        if self.is_linking() && core.person.is_none() {
            return err(link::LINKING);
        }
        if core.person.is_some() && link::link_state(&self.cfg.data_dir).ok().and_then(|s| s.linked_in(ws).cloned()).is_none() {
            return err(link::NOT_LINKED_HERE);
        }
        Ok(())
    }

    /// What the data dir says this install is now: its device, and the
    /// person it acts for if it is linked. `Err` when the link record is
    /// unreadable (it may name a person).
    fn disk_identity(&self) -> std::result::Result<Option<(Uuid, Option<Uuid>)>, ()> {
        let Ok(Some(stored)) = self.identity_store().load() else { return Ok(None) };
        let person = link::link_state(&self.cfg.data_dir).map_err(|_| ())?.person.and_then(|p| p.account_id);
        Ok(Some((stored.device.id, person)))
    }

    fn is_linking(&self) -> bool {
        self.linking.load(std::sync::atomic::Ordering::SeqCst) > 0
    }

    /// Whether `core` signs as an identity the data dir no longer holds (or
    /// can't say: an unreadable link record fails closed).
    fn stale(&self, core: &Core) -> bool {
        !self.is_linking()
            && match self.disk_identity() {
                Ok(d) => d.is_some_and(|d| d != (core.identity.device.id, core.person)),
                Err(()) => true,
            }
    }

    /// Whether the data dir holds a link this open identity doesn't follow
    /// yet. Only a recorded link counts: another lt's redeem switches the
    /// device key before it records the link (and switches back if the
    /// redeem fails), and opening that half-way device as this install's own
    /// member would certify it for the wrong account.
    fn link_to_follow(&self, core: &Core) -> bool {
        !self.is_linking()
            && self
                .disk_identity()
                .ok()
                .flatten()
                .is_some_and(|(device, person)| person.is_some() && (device, person) != (core.identity.device.id, core.person))
    }

    /// Follow the data dir when another lt on it (`lt --plain link-redeem`
    /// while this one is open) switched its identity: reopen as what it holds
    /// now (the new device key, acting as the person), reload the workspace
    /// list, and restart the loops, so this process stops signing as its old
    /// member. A no-op when nothing changed.
    pub(crate) fn follow_disk(self: &Arc<Self>) {
        let Ok(core) = self.core() else { return };
        if !self.link_to_follow(&core) {
            return;
        }
        let _one = lock(&self.following);
        let Ok(core) = self.core() else { return };
        if !self.link_to_follow(&core) {
            return;
        }
        let Ok(Some(stored)) = self.identity_store().load() else { return };
        for (_, stop) in lock(&self.loops).drain() {
            let _ = stop.send(true);
        }
        // Whatever still holds the old service (a turn running, an MCP
        // tool) signs nothing more with it. Never waited on here: a write
        // holding it may be waiting on a passkey.
        core.retired.store(true, std::sync::atomic::Ordering::SeqCst);
        {
            let old = core.clone();
            std::thread::spawn(move || lock(&old.svc).set_write_block(Some(link::STALE.into())));
        }
        // The list first: the new core reads its founders and rosters.
        *lock(&self.list) = WorkspaceList::load(&self.cfg.data_dir, self.secrets.as_ref(), SECRET_SCOPE);
        *self.core.write().unwrap_or_else(|e| e.into_inner()) = None;
        if let Err(e) = self.open_core(stored) {
            for w in lock(&self.list).workspaces.iter() {
                self.emit(Event::Sync { workspace: WorkspaceId::new(w.id().to_string()), status: format!("{}: {}", link::STALE, e.0) });
            }
            return;
        }
        let ids: Vec<Uuid> = lock(&self.list).workspaces.iter().map(|w| w.id()).collect();
        for ws in ids {
            self.activate(ws);
            self.start_loops(ws);
        }
        self.emit(Event::WorkspacesChanged);
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
        self.core().map(|c| c.author.id.clone()).unwrap_or_default()
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
        let linked = link::link_state(&self.cfg.data_dir).ok().and_then(|s| s.linked_in(ws).cloned());
        let mut revoked = false;
        let _ = self.write(ws, |svc| {
            if let Some((f, fk)) = saved.founder_actor_id.as_deref().zip(saved.fk.as_deref()) {
                let _ = svc.store().set_workspace_pin(ws, f, fk);
            }
            // A linked computer whose device was revoked stops writing.
            if linked.is_some() && svc.device_standing(ws, None)? == ligus_runtime::device_enroll::Standing::Revoked {
                svc.set_write_block(Some("this computer was removed from your devices in this workspace (revoked)".into()));
                revoked = true;
                return Ok(());
            }
            let config = workspace_config_session_id(ws);
            if svc.load_in(ws, config)?.is_none() && svc.store().join_invite(ws)?.is_some() {
                return Ok(());
            }
            svc.ensure_self_member(config, ws)?;
            if let Some(ka) = &ka {
                let _ = svc.publish_key_agreement(ws, ka.public_key_bytes());
            }
            let _ = svc.ensure_host(ws, ligus_core::WorkspaceHost::new(device, ligus_core::HostKind::Device, label));
            Ok(())
        });
        if revoked {
            self.emit(Event::Sync {
                workspace: WorkspaceId::new(ws.to_string()),
                status: "revoked: this computer was removed from your devices here".into(),
            });
        }
    }

    fn summary(&self, w: &SavedWorkspace) -> WorkspaceSummary {
        let ws = w.id();
        let me = self.me();
        let role = self
            .read(ws, |svc| Ok(svc.load_in(ws, workspace_config_session_id(ws))?.map(|c| c.role_of(&me))))
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
    fn key_source(&self, w: &SavedWorkspace) -> Result<ligus_runtime::keyring::KeySource> {
        let core = self.core()?;
        let device = core.identity.device.id;
        let ka = self.identity_store().key_agreement(device).map_err(one_line)?;
        Ok(ligus_runtime::keyring::KeySource::for_device(device, ka, w.secrets.key.as_deref(), &w.secrets.ek))
    }

    fn sync_engine(&self, w: &SavedWorkspace) -> Result<ligus_runtime::SyncEngine> {
        let relay = w.relay().ok_or_else(|| BackendError("this workspace has no relay".into()))?;
        let client = ligus_runtime::RelayClient::new(relay).with_auth(w.secrets.token.clone());
        Ok(ligus_runtime::SyncEngine::new(client, w.room.clone()).with_key_source(self.key_source(w)?))
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
            digest_short: ligus_core::keys::hex(&challenge[..8]),
            summary: format!("Create a Ligus passkey for “{ws_name}”"),
            policy: policy.clone(),
        };
        let att = self.presence.create(&req, your_name).map_err(one_line)?;
        let (id, alg, pk) = policy.with(|p| ligus_core::presence::verify_create(&att, &challenge, p)).map_err(|e| BackendError(e.0))?;
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
        let mut redeem = None;
        self.write(ws, |svc| {
            let inv = svc.issue_invite(ws)?;
            // The invite's redeem key: only the code's holder can redeem it
            // (`ligus_core::admission`).
            redeem = svc.take_invite_redeem_seed(ws, inv)?;
            conn.inv = Some(inv.to_string());
            let ek = svc.delivered_epoch_keys(ws, &src)?;
            if !ek.is_empty() {
                conn.ek = ek;
            }
            let (name, ek_json) = (conn.display_name(), conn.ek_json());
            conn.sig = svc.sign_invite(&conn.sig_claims(&name, &ek_json), &cert);
            Ok(())
        })?;
        let code = encode_workspace_invite(&conn);
        match redeem {
            Some(seed) => ligus_runtime::box_admission::with_redeem_key(&code, &seed).map_err(BackendError),
            None => Ok(code),
        }
    }
}

#[async_trait]
impl Backend for RealBackend {
    fn device(&self) -> DeviceInfo {
        let core = self.inner.core().ok();
        let safety_code = core
            .as_ref()
            .and_then(|c| {
                let fp = ligus_runtime::envelope_verifier::FounderPin::fingerprint_hex(&c.identity.account.signing_public_key);
                ligus_runtime::directory_trust::safety_code(&fp)
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
            let your_name = i.core().ok().map(|c| c.author.display_name.clone());
            Ok(StartupState { your_name, workspaces: i.workspaces() })
        })
        .await
    }

    async fn create_workspace(&self, your_name: &str, name: &str) -> Result<Created> {
        self.inner.refuse_if_linked()?;
        let (your_name, name) = (your_name.trim().to_string(), name.trim().to_string());
        if name.is_empty() {
            return err("give the workspace a name");
        }
        let created = self
            .call(move |i| {
                let core = i.ensure_core(&your_name)?;
                let passkey = i.create_passkey(&core, &your_name, &name)?;
                let account = core.identity.account.id;
                let fk = ligus_runtime::envelope_verifier::FounderPin::fingerprint_hex(&core.identity.account.signing_public_key);
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
        self.inner.refuse_if_linked()?;
        let mut conn = decode_workspace_invite(invite).map_err(BackendError)?;
        let redeem = ligus_runtime::box_admission::redeem_key_of(invite);
        if conn.dm_account.is_some() {
            return err(deploy::NO_DMS);
        }
        // The desktop's rule: an invite whose founder key contradicts the one
        // this device's workspaces register for that person isn't from them.
        let checked = conn.clone();
        let (trust, _) = self.call(move |i| Ok(i.invite_trust(&checked))).await?;
        if trust == InviteTrust::Mismatch {
            return err(deploy::MISMATCH);
        }
        if let Some(problem) = invite_relay_problem(&conn.relay_url) {
            return Err(BackendError(problem));
        }
        let ws = conn.id();
        let your_name = your_name.trim().to_string();
        let saved = self
            .call(move |i| {
                // An invite never rewrites a workspace this device holds: its
                // relay, key and founder (lt's trusted founder comes from
                // this list, so a co-member who knows the room could
                // otherwise move the pin, or drop it).
                if lock(&i.list).get(ws).is_some() {
                    return err("this device already has that workspace: an invite can't change its relay, key or founder");
                }
                i.ensure_core(&your_name)?;
                let store = EventStore::open(i.db()).map_err(one_line)?;
                if let Some(inv) = conn.invite_id() {
                    store.set_join_invite(ws, inv).map_err(one_line)?;
                    // Its redeem key proves the redemption.
                    store.set_join_redeem(ws, redeem.as_ref()).map_err(one_line)?;
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
        let joined = self.finish_join(ws, saved).await;
        if joined.is_err() {
            // Nothing half-joined stays behind (it would block a retry).
            let _ = self
                .call(move |i| {
                    let removed = lock(&i.list).remove(ws, i.secrets.as_ref(), SECRET_SCOPE).map_err(one_line)?;
                    if removed {
                        i.save_list()?;
                    }
                    // Its founder pin and invite too: a retry pins again.
                    EventStore::open(i.db()).and_then(|s| s.forget_workspace(ws)).map_err(one_line)?;
                    Ok(())
                })
                .await;
        }
        joined
    }

    async fn link_request(&self) -> Result<LinkRequestView> {
        self.call(|i| i.link_request()).await
    }

    async fn inspect_link_grant(&self, grant: &str) -> Result<LinkGrantView> {
        let grant = grant.trim().to_string();
        self.call(move |i| i.inspect_link_grant(&grant)).await
    }

    async fn redeem_link(&self, grant: &str) -> Result<WorkspaceSummary> {
        self.inner.redeem_link(grant.trim()).await
    }

    fn linked_person(&self) -> Option<String> {
        self.inner.linked_person().map(|p| ligus_runtime::display::shown(&p.display_name))
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
        let code = self.call(move |i| i.invite(ws)).await?;
        self.publish_now(ws).await?;
        Ok(code)
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
            let s = i.read(ws, |svc| svc.load_in(ws, sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
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
        let (message, gate) = self.call(move |i| turns::post_and_plan(i, ws, sid, &text)).await?;
        match gate {
            turns::Gate::Run(plan) => turns::spawn_turn(self.inner.clone(), plan),
            turns::Gate::Defer(plan) => turns::arm_fallback(self.inner.clone(), plan),
            turns::Gate::None => {}
        }
        Ok(message)
    }

    async fn mentionables(&self, workspace: &WorkspaceId) -> Result<Vec<Mentionable>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let me = i.me();
            let config = i.read(ws, |svc| svc.load_in(ws, workspace_config_session_id(ws)))?;
            let mut out = vec![
                Mentionable { handle: "primary".into(), kind: "Default agent".into(), unavailable: None },
                Mentionable { handle: "all".into(), kind: "Everyone".into(), unavailable: None },
            ];
            if let Some(c) = config {
                for a in &c.workspace_agents {
                    let unavailable = i.agent_access(ws, a, false)?.unavailable;
                    out.push(Mentionable { handle: a.name.clone(), kind: "Agent".into(), unavailable });
                }
                out.extend(c.members.iter().filter(|m| m.id != me).map(|m| Mentionable {
                    handle: m.actor.display_name.clone(),
                    kind: views::role(m.role).label().to_string(),
                    unavailable: None,
                }));
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

    async fn stop_turn(&self, _workspace: &WorkspaceId, chat: &ChatId) -> Result<usize> {
        let sid = uuid_of(chat.as_str(), "chat")?;
        Ok(self.inner.turn_stops.stop(sid, None))
    }

    async fn lock_chat(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        self.call(move |i| {
            i.write(ws, |svc| svc.self_medicate(ws, sid, None, &[(ligus_core::guard::LockdownSignal::Manual, "manual".into())]))?;
            Ok(())
        })
        .await
    }

    async fn clear_lockdown(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<()> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        self.call(move |i| {
            let me = i.me();
            let s = i.read(ws, |svc| svc.load_in(ws, sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
            let view = ligus_runtime::lockdown_view::chat_lockdown(&s, sid, &me);
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
            let s = i.read(ws, |svc| svc.load_in(ws, sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
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
            let activity = i.activity(ws)?;
            let hosts = i.read(ws, |svc| svc.list_hosts(ws))?;
            let now = Timestamp::now();
            activity
                .agents
                .iter()
                .map(|a| {
                    let host = i.agent_host(a, &activity.me);
                    let online = host == device || hosts.iter().any(|h| h.id == host && i.beats(ws).is_online(h, &now));
                    let status = if activity.busy.contains(&a.id) {
                        AgentStatus::Busy
                    } else if !host.is_empty() && !online {
                        AgentStatus::Offline
                    } else {
                        AgentStatus::Idle
                    };
                    let doing = activity.tasks.iter().filter(|t| t.active && t.agent == a.name).max_by_key(|t| t.at).map(|t| {
                        let pct = t.progress.map(|p| format!(" {p}%")).unwrap_or_default();
                        format!("{}{pct}: {}", t.status, t.summary)
                    });
                    let access = i.agent_access(ws, a, true)?;
                    Ok(Agent {
                        id: AgentId::new(a.id.to_string()),
                        handle: a.name.clone(),
                        runtime: RuntimeId::new(a.runtime_id.clone()),
                        role: Some(a.role.clone()).filter(|r| !r.is_empty()),
                        host: Some(host).filter(|h| !h.is_empty() && *h != device).map(HostId::new),
                        shared: a.owner_actor_id.is_empty(),
                        status,
                        doing,
                        access: access.label,
                        unavailable: access.unavailable,
                        can_manage: access.can_manage,
                        usage: access.usage,
                    })
                })
                .collect::<Result<Vec<_>>>()
        })
        .await
    }

    async fn runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<Runtime>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let rts = i.read(ws, |svc| svc.list_workspace_runtimes(ws))?;
            let local = i.local_runtimes_in_use(ws, &rts)?;
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
                .chain(local)
                .collect())
        })
        .await
    }

    async fn hosts(&self, workspace: &WorkspaceId) -> Result<Vec<Host>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let device = i.core()?.identity.device.id.to_string();
            let hosts = i.read(ws, |svc| svc.list_hosts(ws))?;
            let now = Timestamp::now();
            let activity = i.activity(ws)?;
            Ok(hosts
                .iter()
                .map(|h| {
                    let agents: Vec<&ligus_core::WorkspaceAgent> =
                        activity.agents.iter().filter(|a| i.agent_host(a, &activity.me) == h.id).collect();
                    Host {
                        id: HostId::new(h.id.clone()),
                        label: if h.id == device { "This device".into() } else { h.label.clone() },
                        kind: match h.kind {
                            ligus_core::HostKind::Device => HostKind::Device,
                            ligus_core::HostKind::Worker => HostKind::Worker,
                        },
                        online: h.id == device || i.beats(ws).is_online(h, &now),
                        last_seen: Some(views::ms(&i.beats(ws).last_seen(h))),
                        this_device: h.id == device,
                        busy: agents.iter().any(|a| activity.busy.contains(&a.id)),
                        agents: agents.iter().map(|a| a.name.clone()).collect(),
                        // Hosts don't publish repo coverage in their beats yet.
                        repos: Vec::new(),
                    }
                })
                .collect())
        })
        .await
    }

    async fn local_runtimes(&self, workspace: &WorkspaceId) -> Result<Vec<LocalRuntime>> {
        let ws = ws_uuid(workspace)?;
        let env = self.inner.cfg.detect_env.clone().unwrap_or_else(detect::process_env);
        let mut found = {
            let env = env.clone();
            tokio::task::spawn_blocking(move || detect::clis(&env)).await.map_err(one_line)?
        };
        if let Some(url) = self.inner.cfg.ollama_url.as_deref() {
            found.extend(detect::ollama(url).await);
        }
        found.extend(detect::api_keys(&env));
        self.call(move |i| {
            let activity = i.activity(ws)?;
            let device = i.core()?.identity.device.id.to_string();
            let rts = i.read(ws, |svc| svc.list_workspace_runtimes(ws))?;
            let taken = i.taken_names(ws)?;
            // An agent here "uses" a runtime found here when it names it, or
            // names a workspace runtime of the same kind (and model).
            let here: Vec<&ligus_core::WorkspaceAgent> =
                activity.agents.iter().filter(|a| i.agent_host(a, &activity.me) == device).collect();
            Ok(found
                .into_iter()
                .map(|f| {
                    let used_by = here
                        .iter()
                        .filter(|a| {
                            a.runtime_id == f.id
                                || rts.iter().any(|r| {
                                    r.id == a.runtime_id
                                        && r.provider == f.provider
                                        && f.model.as_deref().is_none_or(|m| r.model.is_empty() || r.model == m)
                                })
                        })
                        .map(|a| a.name.clone())
                        .collect();
                    LocalRuntime {
                        id: RuntimeId::new(f.id),
                        kind: views::runtime_kind(f.provider),
                        model: f.model,
                        version: f.version,
                        ready: f.ready,
                        detail: f.detail,
                        used_by,
                        unsupported: f.unsupported,
                        suggested_name: detect::unique_name(&f.suggested, &taken),
                    }
                })
                .collect())
        })
        .await
    }

    async fn add_local_agent(&self, workspace: &WorkspaceId, runtime: &RuntimeId, name: &str) -> Result<String> {
        let ws = ws_uuid(workspace)?;
        let runtime = runtime.0.clone();
        let name = name.trim().trim_start_matches('@').to_string();
        if detect::parse_local_id(&runtime).is_none() {
            return Err(BackendError(format!("{runtime} isn't a runtime on this device")));
        }
        self.call(move |i| {
            i.add_agent_here(ws, &name, runtime)?;
            Ok(name)
        })
        .await
    }

    async fn set_agent_access(&self, workspace: &WorkspaceId, agent: &AgentId, everyone: bool) -> Result<()> {
        let (ws, aid) = (ws_uuid(workspace)?, uuid_of(agent.as_str(), "agent")?);
        self.call(move |i| {
            let a = i
                .read(ws, |svc| svc.load_in(ws, workspace_config_session_id(ws)))?
                .and_then(|c| c.workspace_agents.into_iter().find(|a| a.id == aid))
                .ok_or_else(|| BackendError("no such agent".into()))?;
            // Only who may use it changes; its budgets and notice stay.
            let mut policy = a.policy();
            policy.access = if everyone { ligus_core::agent::ACCESS_EVERYONE } else { ligus_core::agent::ACCESS_OWNER }.into();
            i.write(ws, |svc| svc.set_agent_policy(ws, aid, policy))?;
            i.emit(Event::AgentsChanged { workspace: WorkspaceId::new(ws.to_string()) });
            Ok(())
        })
        .await
    }

    async fn set_agent_runtime(&self, workspace: &WorkspaceId, agent: &AgentId, runtime: &RuntimeId) -> Result<()> {
        let (ws, aid) = (ws_uuid(workspace)?, uuid_of(agent.as_str(), "agent")?);
        let runtime = runtime.0.clone();
        self.call(move |i| {
            let config = workspace_config_session_id(ws);
            let mut a = i
                .read(ws, |svc| svc.load_in(ws, config))?
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
            Ok(views::members(&config, timeline.current(), &me))
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

    async fn inspect_invite(&self, invite: &str) -> Result<InviteCheck> {
        let invite = invite.to_string();
        self.call(move |i| i.inspect(&invite)).await
    }

    async fn connect_deployment(&self, your_name: &str, spec: DeploymentSpec) -> Result<Created> {
        self.inner.refuse_if_linked()?;
        let your_name = your_name.trim().to_string();
        // Check what can be checked before touching the identity or the relay.
        deploy::deployment_conn(&spec, String::new(), String::new())?;
        let saved = self
            .call({
                let your_name = your_name.clone();
                let spec = spec.clone();
                move |i| {
                    let core = i.ensure_core(&your_name)?;
                    let fk = ligus_runtime::envelope_verifier::FounderPin::fingerprint_hex(&core.identity.account.signing_public_key);
                    let conn = deploy::deployment_conn(&spec, core.identity.actor().id, fk)?;
                    if lock(&i.list).get(conn.id()).is_some() {
                        return err("this device already has that workspace");
                    }
                    Ok(SavedWorkspace::from_conn(&conn, spec.token.clone().filter(|t| !t.trim().is_empty())))
                }
            })
            .await?;
        // A box never founds a workspace, and neither does lt over one that
        // exists: that needs an invite from its Admin.
        if deploy::relay_has_workspace(&self.inner, &saved).await? {
            return err("the relay already holds this workspace: join it with an invite from one of its Admins");
        }
        let created = self
            .call(move |i| {
                let core = i.core()?;
                let passkey = i.create_passkey(&core, &your_name, &saved.name)?;
                let account = core.identity.account.id;
                let ws = saved.id();
                lock(&i.list).upsert(saved.clone());
                i.save_list()?;
                let key = RecoveryKey::generate().map_err(|e| BackendError(e.0))?;
                i.write(ws, |svc| {
                    let config = svc.ensure_workspace_config(ws)?;
                    svc.ensure_self_member(config, ws)?;
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

    async fn inspect_join_request(&self, join_request: &str) -> Result<JoinRequestCheck> {
        deploy::inspect_join_request(join_request)
    }

    async fn admit_box(&self, workspace: &WorkspaceId, join_request: Option<&str>) -> Result<BoxAdmission> {
        let ws = ws_uuid(workspace)?;
        let request = join_request.map(str::to_string);
        let admission = self.call(move |i| i.admit(ws, request.as_deref())).await?;
        self.publish_now(ws).await?;
        Ok(admission)
    }

    async fn handoffs(&self, workspace: &WorkspaceId) -> Result<Vec<Handoff>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let mut out: Vec<Handoff> = i.chat_sessions(ws)?.iter().flat_map(views::handoffs).collect();
            out.sort_by_key(|x| std::cmp::Reverse(x.at));
            out.truncate(50);
            Ok(out)
        })
        .await
    }

    async fn checkout(&self, workspace: &WorkspaceId) -> Result<Option<String>> {
        let id = workspace.0.clone();
        self.call(move |i| Ok(lock(&i.local).checkouts.get(&id).cloned())).await
    }

    async fn repo_choices(&self, workspace: &WorkspaceId) -> Result<RepoChoices> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| scratch::choices(i, ws)).await
    }

    async fn set_chat_repo(&self, workspace: &WorkspaceId, chat: &ChatId, repo: Option<&str>) -> Result<()> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        let repo = repo.map(str::to_string);
        self.call(move |i| scratch::set_chat_repo(i, ws, sid, repo.as_deref())).await?;
        self.inner.emit(Event::ChatsChanged { workspace: workspace.clone() });
        Ok(())
    }

    async fn new_scratch_repo(&self, workspace: &WorkspaceId, chat: &ChatId) -> Result<ChatRepo> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        let repo = self.call(move |i| scratch::create(i, ws, sid)).await?;
        self.inner.emit(Event::ChatsChanged { workspace: workspace.clone() });
        Ok(repo)
    }

    async fn keep_scratch_repo(&self, workspace: &WorkspaceId, chat: &ChatId, dest: &str) -> Result<String> {
        let (ws, sid) = (ws_uuid(workspace)?, uuid_of(chat.as_str(), "chat")?);
        let dest = dest.to_string();
        let path = self.call(move |i| scratch::keep(i, ws, sid, &dest)).await?;
        self.inner.emit(Event::ChatsChanged { workspace: workspace.clone() });
        Ok(path)
    }

    async fn set_checkout(&self, workspace: &WorkspaceId, path: Option<&str>) -> Result<Option<String>> {
        ws_uuid(workspace)?;
        let (workspace, path) = (workspace.clone(), path.map(str::to_string));
        let bound = self.call(move |i| i.bind(&workspace, path.as_deref().map(Path::new))).await?;
        Ok(bound)
    }

    async fn workflows(&self, workspace: &WorkspaceId) -> Result<Vec<Workflow>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let me = i.me();
            let mut out: Vec<Workflow> = i
                .chat_sessions(ws)?
                .iter()
                .flat_map(|s| s.workflow_definitions.iter().map(|d| views::workflow(s, d, &me)).collect::<Vec<_>>())
                .collect();
            out.sort_by_key(|x| x.name.to_lowercase());
            Ok(out)
        })
        .await
    }

    async fn runs(&self, workspace: &WorkspaceId) -> Result<Vec<Run>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let mut out: Vec<Run> =
                i.chat_sessions(ws)?.iter().flat_map(|s| s.workflow_runs.iter().map(|r| i.run_view(s, r)).collect::<Vec<_>>()).collect();
            out.sort_by_key(|x| std::cmp::Reverse(x.started_at));
            Ok(out)
        })
        .await
    }

    async fn start_run(&self, workspace: &WorkspaceId, workflow: &WorkflowId, input: &str) -> Result<Run> {
        let (ws, def) = (ws_uuid(workspace)?, uuid_of(workflow.as_str(), "workflow")?);
        let input = input.to_string();
        self.call(move |i| workflows::start(i, ws, def, &input)).await
    }

    async fn cancel_run(&self, workspace: &WorkspaceId, run: &RunId) -> Result<Run> {
        let (ws, rid) = (ws_uuid(workspace)?, uuid_of(run.as_str(), "run")?);
        self.call(move |i| workflows::cancel(i, ws, rid)).await?;
        // The driver settles it on its next round.
        for _ in 0..50 {
            let r = self.call(move |i| workflows::find_run(i, ws, rid).map(|(r, s)| i.run_view(&s, &r))).await?;
            if r.status.is_terminal() {
                return Ok(r);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.call(move |i| workflows::find_run(i, ws, rid).map(|(r, s)| i.run_view(&s, &r))).await
    }

    async fn decide_gate(&self, workspace: &WorkspaceId, run: &RunId, stage: &str, approve: bool) -> Result<Run> {
        let (ws, rid) = (ws_uuid(workspace)?, uuid_of(run.as_str(), "run")?);
        let stage = stage.to_string();
        self.call(move |i| {
            let (r, s) = workflows::find_run(i, ws, rid)?;
            let node = r.nodes.iter().find(|n| n.node_id == stage).ok_or_else(|| BackendError("no such stage".into()))?;
            let pid = match (node.proposal_id, node.status) {
                (Some(p), ligus_core::workflow::NodeRunStatus::AwaitingApproval) => p,
                _ => return err("that stage isn't a gate waiting for a decision"),
            };
            i.write(ws, |svc| svc.vote_on_proposal(s.id, ws, pid, approve))?;
            workflows::wake_all(i, ws);
            let (r, s) = workflows::find_run(i, ws, rid)?;
            Ok(i.run_view(&s, &r))
        })
        .await
    }

    async fn memory(&self, workspace: &WorkspaceId) -> Result<Memory> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| Ok(views::memory(&i.read(ws, |svc| svc.memory(ws))?))).await
    }

    async fn task_state(&self, workspace: &WorkspaceId) -> Result<Vec<TaskState>> {
        let ws = ws_uuid(workspace)?;
        self.call(move |i| {
            let now = ligus_core::flow::now_ms();
            let mut out: Vec<TaskState> = i.chat_sessions(ws)?.iter().flat_map(|s| views::task_state(s, now)).collect();
            out.sort_by_key(|x| std::cmp::Reverse(x.at));
            Ok(out)
        })
        .await
    }

    async fn setup_status(&self, workspace: Option<&WorkspaceId>) -> Result<Vec<crate::setup::FlowRow>> {
        let ws = workspace.map(ws_uuid).transpose()?;
        let device = crate::setup::device_row(self.linked_person());
        self.call(move |i| Ok(crate::setup::rows(&setup_ops::RealOps::new(i.clone(), ws), Some(device)))).await
    }

    async fn setup_form(
        &self,
        workspace: Option<&WorkspaceId>,
        flow: crate::setup::LtFlow,
        inputs: crate::setup::SetupInputs,
    ) -> Result<crate::setup::SetupForm> {
        let crate::setup::LtFlow::Engine(f) = flow else { return Ok(crate::setup::device_form()) };
        let ws = workspace.map(ws_uuid).transpose()?;
        self.call(move |i| Ok(crate::setup::form(f, &setup_ops::RealOps::new(i.clone(), ws), &inputs))).await
    }

    async fn setup_run(
        &self,
        workspace: Option<&WorkspaceId>,
        flow: crate::setup::LtFlow,
        inputs: crate::setup::SetupInputs,
        secrets: crate::setup::SetupSecrets,
        dry_run: bool,
        cancel: crate::setup::CancelFlag,
    ) -> Result<crate::setup::SetupOutcome> {
        let crate::setup::LtFlow::Engine(f) = flow else { return Ok(crate::setup::device_run(self.linked_person())) };
        let ws = workspace.map(ws_uuid).transpose()?;
        self.call(move |i| {
            let mut ops = setup_ops::RealOps::new(i.clone(), ws);
            Ok(crate::setup::run(f, &mut ops, &inputs, secrets, dry_run, &cancel))
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
            your_name: self.core().map(|c| c.author.display_name.clone()).unwrap_or_default(),
            device_name: self.cfg.device_name.clone(),
            notify_mentions: l.notify_mentions,
            agents_may_edit: l.agents_may_edit,
            relay_url: self.cfg.relay_url.clone(),
            // The person as the open core acts for them (their roster name),
            // falling back to the link record's.
            linked_to: match self.core() {
                Ok(c) if c.person.is_some() => Some(ligus_runtime::display::shown(&c.author.display_name)),
                _ => self.linked_person().map(|p| ligus_runtime::display::shown(&p.display_name)),
            },
        }
    }

    /// The chat sessions of `ws` (never its config log).
    pub(crate) fn chat_sessions(&self, ws: Uuid) -> Result<Vec<ligus_core::ChatSession>> {
        let config = workspace_config_session_id(ws);
        self.read(ws, |svc| {
            let mut out = Vec::new();
            for sid in svc.store().sessions_of_workspace(ws)? {
                if sid == config {
                    continue;
                }
                if let Some(s) = svc.load_in(ws, sid)?
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
        let repos = self.read(ws, |svc| Ok(scratch::repo_views(self, svc, ws))).unwrap_or_default();
        let mut out: Vec<ChatSummary> = self
            .chat_sessions(ws)?
            .iter()
            .map(|s| {
                let id = s.id.to_string();
                let last = s.messages.iter().rev().find(|m| m.role != ligus_core::MessageRole::System);
                let updated_at = last.map(|m| views::ms(&m.created_at)).unwrap_or_else(|| views::ms(&s.created_at));
                let seen = read.get(&id).copied().unwrap_or(0);
                let unread = last.is_some_and(|m| views::ms(&m.created_at) > seen && m.actor_identity.as_ref().is_none_or(|a| a.id != me));
                let view = ligus_runtime::lockdown_view::chat_lockdown(s, s.id, &me);
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
                    repo: s.repo.as_ref().and_then(|r| r.repo_id).and_then(|r| repos.get(&r).cloned()),
                }
            })
            .collect();
        out.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
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
        out.sort_by_key(|x| std::cmp::Reverse(x.created_at));
        Ok(out)
    }

    fn find_proposal(&self, ws: Uuid, pid: Uuid) -> Result<(Uuid, ligus_core::ActionProposal)> {
        self.chat_sessions(ws)?
            .into_iter()
            .find_map(|s| s.proposals.iter().find(|p| p.id == pid).cloned().map(|p| (s.id, p)))
            .ok_or_else(|| BackendError("no such proposal".into()))
    }

    pub(crate) fn proposal_view(&self, ws: Uuid, sid: Uuid, pid: Uuid) -> Result<Proposal> {
        let me = self.me();
        let hosts = self.host_labels(ws);
        let s = self.read(ws, |svc| svc.load_in(ws, sid))?.ok_or_else(|| BackendError("no such chat".into()))?;
        let p = s.proposals.iter().find(|p| p.id == pid).ok_or_else(|| BackendError("no such proposal".into()))?;
        Ok(views::proposal(&s, p, &me, &hosts))
    }
}

impl Inner {
    /// Who is busy and what agents say they are doing, in `ws`.
    fn activity(&self, ws: Uuid) -> Result<Activity> {
        let now = ligus_core::flow::now_ms();
        let config = self.read(ws, |svc| svc.load_in(ws, workspace_config_session_id(ws)))?;
        let agents = config.map(|c| c.workspace_agents).unwrap_or_default();
        let mut busy = std::collections::HashSet::new();
        let mut tasks = Vec::new();
        let cutoff = views::ms(&Timestamp::now()) - 600_000;
        for s in self.chat_sessions(ws)? {
            // serve's rule: a reply streaming for under ten minutes.
            for m in s.messages.iter().rev().take(20) {
                if m.is_streaming
                    && views::ms(&m.created_at) >= cutoff
                    && let Some(r) = m.responder_id
                {
                    busy.insert(r);
                }
            }
            tasks.extend(views::task_state(&s, now));
        }
        Ok(Activity { me: self.me(), agents, busy, tasks })
    }

    /// Agent access controls as this device's person sees `a`: who may use
    /// it, whether they may (and why not), whether they manage it, and
    /// (`usage`, for an agent they manage) this device's counts.
    fn agent_access(&self, ws: Uuid, a: &ligus_core::WorkspaceAgent, usage: bool) -> Result<AgentAccessView> {
        use ligus_runtime::agent_access::{access_label, availability, person_role};
        self.read(ws, |svc| {
            let local = svc.is_local_workspace(ws);
            let links = svc.device_links(ws).unwrap_or_default();
            let me = svc.author().id.clone();
            let unavailable = if local {
                None
            } else {
                let roster = svc.workspace_roster(ws).ok();
                availability(a, &links, &me, person_role(roster.as_ref(), &links, links.person(&me)))
            };
            let can_manage = svc.may_manage_agent(ws, a);
            let usage = if usage && can_manage {
                let view = svc.agent_usage_view(a.id)?;
                let config = svc.load_in(ws, workspace_config_session_id(ws))?;
                let name = |member: &str| -> String {
                    if member.is_empty() {
                        return "Unattributed".into();
                    }
                    config
                        .as_ref()
                        .and_then(|c| c.members.iter().find(|m| m.id == member || links.person(&m.id) == member))
                        .map(|m| m.actor.display_name.clone())
                        .unwrap_or_else(|| member.to_string())
                };
                let rows = |rows: &[ligus_runtime::event_store::AgentUsageRow]| -> Vec<UsageRow> {
                    rows.iter()
                        .map(|r| UsageRow {
                            member: name(&r.member),
                            turns: r.turns,
                            uncosted: r.uncosted,
                            cost_micro_usd: (r.turns > r.uncosted).then(|| (r.cost_usd * 1_000_000.0).round() as u64),
                        })
                        .collect()
                };
                Some(AgentUsage { day: view.day.clone(), today: rows(&view.today), month: rows(&view.month) })
            } else {
                None
            };
            Ok(AgentAccessView { label: access_label(a).to_string(), unavailable, can_manage, usage })
        })
    }

    /// The host an agent's turns run on: its bound host, else its owner's
    /// device when that is this device (empty: some other owner's device).
    /// The names a new agent may not take: every agent's and member's.
    /// Add an agent on a runtime this device runs: the desktop's
    /// add-agent, owned by you and only yours to use. An Admin pins it to
    /// this device; a Contributor's runs on its owner's device (an
    /// unpinned agent), which is this one. Its id.
    fn add_agent_here(&self, ws: Uuid, name: &str, runtime: String) -> Result<Uuid> {
        if let Some(why) = detect::name_refusal(name, &self.taken_names(ws)?) {
            return Err(BackendError(why));
        }
        let device = self.core()?.identity.device.id.to_string();
        let me = self.me();
        let role = self.read(ws, |svc| Ok(svc.load_in(ws, workspace_config_session_id(ws))?.map(|c| c.role_of(&me))))?;
        let mut agent = ligus_core::WorkspaceAgent::new(name.to_string(), runtime);
        agent.owner_actor_id = me;
        agent.access = ligus_core::agent::ACCESS_OWNER.into();
        if role.is_some_and(|r| views::role(r) >= Role::Admin) {
            agent.host_id = device;
        }
        let id = agent.id;
        self.write(ws, |svc| svc.add_agent(workspace_config_session_id(ws), ws, agent))?;
        self.emit(Event::AgentsChanged { workspace: WorkspaceId::new(ws.to_string()) });
        Ok(id)
    }

    fn taken_names(&self, ws: Uuid) -> Result<Vec<String>> {
        let config = self.read(ws, |svc| svc.load_in(ws, workspace_config_session_id(ws)))?;
        Ok(config
            .map(|c| {
                c.workspace_agents.iter().map(|a| a.name.clone()).chain(c.members.iter().map(|m| m.actor.display_name.clone())).collect()
            })
            .unwrap_or_default())
    }

    /// The device-local runtimes (`local:…`) the workspace's agents name and
    /// no workspace runtime shadows, for the runtimes list and picker.
    fn local_runtimes_in_use(&self, ws: Uuid, rts: &[ligus_core::WorkspaceRuntime]) -> Result<Vec<Runtime>> {
        let mut ids: Vec<String> =
            self.activity(ws)?.agents.iter().map(|a| a.runtime_id.clone()).filter(|id| !rts.iter().any(|r| &r.id == id)).collect();
        ids.sort();
        ids.dedup();
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                let (provider, model) = detect::parse_local_id(&id)?;
                let kind = views::runtime_kind(provider);
                Some(Runtime {
                    label: format!("{} (this device)", kind.label()),
                    kind,
                    model: Some(model).filter(|m| !m.is_empty()),
                    configured: detect::key_var(provider).is_none_or(|v| std::env::var_os(v).is_some_and(|k| !k.is_empty())),
                    active: false,
                    id: RuntimeId::new(id),
                })
            })
            .collect())
    }

    fn agent_host(&self, a: &ligus_core::WorkspaceAgent, me: &str) -> String {
        if !a.host_id.is_empty() {
            return a.host_id.clone();
        }
        if a.owner_actor_id.is_empty() || a.owner_actor_id == me {
            return self.core().map(|c| c.identity.device.id.to_string()).unwrap_or_default();
        }
        String::new()
    }

    /// The host beats read from `ws`'s room (a shared handle).
    pub(crate) fn beats(&self, ws: Uuid) -> ligus_runtime::HostPresence {
        lock(&self.beats).entry(ws).or_default().clone()
    }

    /// Whether `host` (a host id) has a heartbeat inside the online window.
    pub(crate) fn host_online(&self, ws: Uuid, host: &str) -> bool {
        let now = Timestamp::now();
        self.read(ws, |svc| svc.list_hosts(ws)).unwrap_or_default().iter().any(|h| h.id == host && self.beats(ws).is_online(h, &now))
    }

    /// Bind (or unbind) the checkout this device applies `workspace`'s
    /// proposals in. A bound path must be a git checkout.
    fn bind(&self, workspace: &WorkspaceId, repo: Option<&Path>) -> Result<Option<String>> {
        let path = match repo {
            Some(p) => {
                let p = expand_home(p);
                if !p.join(".git").exists() {
                    return err(format!("{} isn't a git checkout", p.display()));
                }
                Some(std::fs::canonicalize(&p).unwrap_or(p).display().to_string())
            }
            None => None,
        };
        {
            let mut l = lock(&self.local);
            match &path {
                Some(p) => l.checkouts.insert(workspace.0.clone(), p.clone()),
                None => l.checkouts.remove(&workspace.0),
            };
            l.save(&self.cfg.data_dir).map_err(one_line)?;
        }
        self.emit(Event::HostsChanged { workspace: workspace.clone() });
        Ok(path)
    }
}

/// `~/x` as `$HOME/x`.
fn expand_home(p: &Path) -> PathBuf {
    match (p.strip_prefix("~"), std::env::var_os("HOME")) {
        (Ok(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => p.to_path_buf(),
    }
}

/// What [`Inner::agent_access`] returns.
struct AgentAccessView {
    label: String,
    unavailable: Option<String>,
    can_manage: bool,
    usage: Option<AgentUsage>,
}

struct Activity {
    me: String,
    agents: Vec<ligus_core::WorkspaceAgent>,
    busy: std::collections::HashSet<Uuid>,
    tasks: Vec<TaskState>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Starting a workspace's loops runs on an async worker: it must never
    /// wait on the write service, which a write holds for as long as a
    /// passkey prompt is open (up to 180 s). Shown structurally, not by
    /// timing: start_loops returns while another thread holds the lock.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn starting_the_loops_never_waits_on_a_write() {
        // SAFETY: tests in this binary only ever set it to the same value.
        unsafe { std::env::set_var("LIGUS_SECRET_BACKEND", "file") };
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = RealConfig::new(tmp.path().join("dev"));
        cfg.cli_fallback = false;
        let b = RealBackend::open(cfg).unwrap();
        let inner = b.inner.clone();
        let core = tokio::task::spawn_blocking({
            let i = inner.clone();
            move || i.ensure_core("Ana").map_err(|e| e.0)
        })
        .await
        .unwrap()
        .unwrap();
        let conn = WorkspaceConn {
            name: "W".into(),
            relay_url: "http://127.0.0.1:9".into(),
            room: random_token(32),
            key: Some(random_token(24)),
            icon: None,
            dm_account: None,
            founder_actor_id: None,
            fk: None,
            inv: None,
            ek: Vec::new(),
            sig: None,
        };
        let ws = conn.id();
        lock(&inner.list).upsert(SavedWorkspace::from_conn(&conn, None));
        // A write waiting on a passkey: the service stays locked until
        // start_loops has returned (or, if it is waiting on the lock, for a
        // minute, so the test fails instead of hanging).
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let svc = core.svc.clone();
        let holder = std::thread::spawn(move || {
            let _g = svc.lock().unwrap();
            locked_tx.send(()).unwrap();
            // Whether start_loops returned while the lock was held.
            started_rx.recv_timeout(Duration::from_secs(60)).is_ok()
        });
        locked_rx.recv().unwrap();
        inner.start_loops(ws);
        let _ = started_tx.send(());
        let returned_while_locked = holder.join().unwrap();
        b.shutdown();
        assert!(returned_while_locked, "start_loops waited on the write lock");
    }

    /// Shutting down stops the turns running here, and a turn that starts
    /// afterwards is born stopped (the engine returns before it begins).
    #[tokio::test]
    async fn shutdown_stops_the_turns_running_here() {
        // SAFETY: tests in this binary only ever set it to the same value.
        unsafe { std::env::set_var("LIGUS_SECRET_BACKEND", "file") };
        let tmp = tempfile::tempdir().unwrap();
        let b = RealBackend::open(RealConfig::new(tmp.path().join("dev"))).unwrap();
        let chat = Uuid::new_v4();
        let running = b.inner.turn_stops.register(chat);
        assert!(!running.signal().is_stopped());
        b.shutdown();
        assert!(running.signal().is_stopped());
        assert!(b.inner.turn_stops.register(Uuid::new_v4()).signal().is_stopped());
    }

    /// Stop reaches only the turns running here in that chat, and says how
    /// many it stopped.
    #[tokio::test]
    async fn stop_turn_stops_this_chats_turns_here() {
        // SAFETY: tests in this binary only ever set it to the same value.
        unsafe { std::env::set_var("LIGUS_SECRET_BACKEND", "file") };
        let tmp = tempfile::tempdir().unwrap();
        let b = RealBackend::open(RealConfig::new(tmp.path().join("dev"))).unwrap();
        let (chat, other) = (Uuid::new_v4(), Uuid::new_v4());
        let running = b.inner.turn_stops.register(chat);
        let elsewhere = b.inner.turn_stops.register(other);
        let ws = WorkspaceId::new(Uuid::new_v4().to_string());
        assert_eq!(b.stop_turn(&ws, &ChatId::new(other.to_string())).await.unwrap(), 1);
        assert!(!running.signal().is_stopped() && elsewhere.signal().is_stopped());
        assert_eq!(b.stop_turn(&ws, &ChatId::new(Uuid::new_v4().to_string())).await.unwrap(), 0);
        assert!(b.stop_turn(&ws, &ChatId::new("not-a-chat")).await.is_err());
        b.shutdown();
    }
}
