//! Live updates and the background loops.
//!
//! - **Translator.** Subscribes to the write service's change feed
//!   (`ChatService::subscribe`), which carries this device's appends and,
//!   through the sync loops, what each pull brought in. It reloads each
//!   changed session from the read service, compares it with what it last
//!   saw, and emits `lt`'s events: new messages, streamed text, finished
//!   replies, proposal changes, chat-list, member, agent and host changes,
//!   and mention notifications. User messages that arrived through sync go
//!   to the relay-sync dispatch gate.
//! - **Sync loop.** `ligus_runtime::sync_service::SyncLoop` per workspace
//!   with a relay.
//! - **Host loop.** Heartbeats this device as a host and runs the
//!   apply-on-host loop (`ligus_runtime::sync_service::apply_tick`) when a
//!   checkout is bound.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use ligus_core::{ChatSession, MessageRole, workspace_config_session_id};
use ligus_runtime::chat_service::{ChangeOrigin, ChatChange};
use ligus_runtime::sync_service::{SyncLoop, SyncStatus};
use tokio::sync::broadcast;
use uuid::Uuid;

use super::{Core, Inner, lock, turns, views};
use crate::backend::Event;
use crate::backend::model::*;

/// What the translator last saw of one session.
#[derive(Default, Clone)]
pub struct Snap {
    title: String,
    channel: String,
    /// (message, body length, streaming)
    messages: Vec<(Uuid, usize, bool)>,
    proposals: HashMap<Uuid, String>,
    lockdowns: usize,
    /// For the config log: members, agents, hosts.
    config: (String, String, String),
    /// Workflow runs: id to (status, stages, epoch).
    runs: HashMap<Uuid, String>,
    /// Workflow definitions.
    workflows: String,
    /// Memory (config log) and task state and hand-offs (chats).
    flow: String,
}

#[derive(Default)]
pub struct Snaps(HashMap<Uuid, Snap>);

fn snap_of(s: &ChatSession) -> Snap {
    Snap {
        title: s.title.clone(),
        channel: s.channel_id.clone(),
        messages: s.messages.iter().map(|m| (m.id, m.body.len(), m.is_streaming)).collect(),
        proposals: s.proposals.iter().map(|p| (p.id, proposal_key(s, p))).collect(),
        lockdowns: s.guards.lockdowns_of(s.id).len(),
        config: (
            format!("{:?}", s.members.iter().map(|m| (&m.id, m.role)).collect::<Vec<_>>()),
            format!("{:?}", s.workspace_agents.iter().map(|a| (a.id, &a.runtime_id, &a.host_id, &a.name)).collect::<Vec<_>>()),
            format!("{:?}", s.workspace_hosts.iter().map(|h| (&h.id, &h.label, h.last_seen.inner().unix_timestamp())).collect::<Vec<_>>()),
        ),
        runs: s
            .workflow_runs
            .iter()
            .map(|r| {
                (
                    r.id,
                    format!(
                        "{:?}/{:?}/{:?}",
                        r.status,
                        r.nodes.iter().map(|n| (n.status, n.proposal_id)).collect::<Vec<_>>(),
                        r.lease_epoch
                    ),
                )
            })
            .collect(),
        workflows: format!("{:?}", s.workflow_definitions.iter().map(|d| (d.id, d.disabled, d.nodes.len())).collect::<Vec<_>>()),
        flow: format!(
            "{}/{:?}/{}/{}",
            s.flow.memory.entries.len(),
            s.flow.memory.entries.values().map(|e| e.current.rev).collect::<Vec<_>>(),
            s.flow.tasks.iter().map(|t| t.rev.to_string()).collect::<String>(),
            s.flow.handoffs.len()
        ),
    }
}

fn proposal_key(s: &ChatSession, p: &ligus_core::ActionProposal) -> String {
    let requests = s.apply_requests.values().filter(|r| r.proposal_id == p.id).count();
    let outcomes = s.apply_requests.values().filter(|r| r.proposal_id == p.id && s.apply_outcomes.contains_key(&r.id)).count();
    format!("{:?}/{}/{}/{}/{}/{:?}", p.status, p.approvals.len(), requests, outcomes, p.dismissed, p.archive_mark)
}

/// Remember every session as it is now, so only later changes are events.
pub fn prime(inner: &Inner, core: &Core) {
    let r = lock(&core.reader);
    let Ok(ids) = r.store().list_session_ids() else { return };
    let mut snaps = lock(&inner.snaps);
    for sid in ids {
        if let Ok(Some(s)) = r.load_resolving(sid) {
            snaps.0.insert(sid, snap_of(&s));
        }
    }
}

/// Translate the change feed into events, in batches.
pub fn spawn_translator(inner: Arc<Inner>, mut rx: broadcast::Receiver<ChatChange>) {
    tokio::spawn(async move {
        loop {
            let mut batch: HashMap<Uuid, (Uuid, bool)> = HashMap::new();
            let add = |c: ChatChange, batch: &mut HashMap<Uuid, (Uuid, bool)>| {
                let e = batch.entry(c.session_id).or_insert((c.workspace_id, false));
                e.1 |= c.origin == ChangeOrigin::Synced;
            };
            match rx.recv().await {
                Ok(c) => add(c, &mut batch),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    inner.emit(Event::WorkspacesChanged);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
            // Coalesce a burst (a streamed reply appends many chunks).
            tokio::time::sleep(Duration::from_millis(30)).await;
            while let Ok(c) = rx.try_recv() {
                add(c, &mut batch);
            }
            let i = inner.clone();
            let triggers = tokio::task::spawn_blocking(move || diff(&i, batch)).await.unwrap_or_default();
            for (ws, sid, trigger) in triggers {
                let i = inner.clone();
                tokio::spawn(async move {
                    let plan = tokio::task::spawn_blocking({
                        let i = i.clone();
                        move || turns::plan_synced(&i, ws, sid, trigger)
                    })
                    .await;
                    if let Ok(Ok(Some(plan))) = plan {
                        turns::spawn_turn(i, plan);
                    }
                });
            }
        }
    });
}

/// Emit what changed in `batch`; return the synced user messages to offer
/// the relay-sync gate.
fn diff(inner: &Inner, batch: HashMap<Uuid, (Uuid, bool)>) -> Vec<(Uuid, Uuid, Uuid)> {
    let mut triggers = Vec::new();
    let Ok(core) = inner.core() else { return triggers };
    let me = core.author.id.clone();
    let my_device = core.identity.device.id;
    let known: HashSet<Uuid> = lock(&inner.list).workspaces.iter().map(|w| w.id()).collect();
    for (sid, (ws, synced)) in batch {
        if !known.contains(&ws) {
            continue;
        }
        let Ok(Some(s)) = inner.read(ws, |svc| svc.load_in(ws, sid)) else { continue };
        let wsid = WorkspaceId::new(ws.to_string());
        let old = lock(&inner.snaps).0.get(&sid).cloned();
        let new = snap_of(&s);
        lock(&inner.snaps).0.insert(sid, new.clone());
        let old_flow = old.as_ref().map(|o| o.flow.clone()).unwrap_or_default();
        for d in lock(&inner.drivers).values().filter(|d| d.sid == sid) {
            d.wake.notify_one();
        }
        if sid == workspace_config_session_id(ws) {
            let o = old.map(|o| o.config).unwrap_or_default();
            if o.0 != new.config.0 {
                inner.emit(Event::MembersChanged { workspace: wsid.clone() });
                inner.emit(Event::WorkspacesChanged);
            }
            if o.1 != new.config.1 {
                inner.emit(Event::AgentsChanged { workspace: wsid.clone() });
            }
            if o.2 != new.config.2 {
                inner.emit(Event::HostsChanged { workspace: wsid.clone() });
            }
            if old_flow != new.flow {
                inner.emit(Event::MemoryChanged { workspace: wsid.clone() });
            }
            continue;
        }
        let old = old.unwrap_or_default();
        let fresh = old.messages.is_empty() && old.title.is_empty();
        if fresh || old.title != new.title || old.channel != new.channel || old.lockdowns != new.lockdowns {
            inner.emit(Event::ChatsChanged { workspace: wsid.clone() });
        }
        let before: HashMap<Uuid, (usize, bool)> = old.messages.iter().map(|(id, len, st)| (*id, (*len, *st))).collect();
        let mut any_new = false;
        for m in &s.messages {
            // A superseded answer is hidden, as the transcript hides it.
            if m.superseded {
                continue;
            }
            match before.get(&m.id) {
                None => {
                    any_new = true;
                    if m.is_streaming {
                        let mut sent = lock(&inner.emitted);
                        let e = sent.entry(m.id).or_insert(0);
                        *e = (*e).max(m.body.len());
                    }
                    inner.emit(Event::MessageAdded { workspace: wsid.clone(), message: views::message(&s, m) });
                    let mine = m.origin_device_id == Some(my_device) || m.actor_identity.as_ref().is_some_and(|a| a.id == me);
                    if synced && m.role == MessageRole::User && !mine {
                        triggers.push((ws, sid, m.id));
                    }
                    if synced && !mine && m.role != MessageRole::System && !m.is_streaming {
                        notify_if_mentioned(inner, &s, m, &me);
                    }
                }
                Some((len, streaming)) => {
                    let from = {
                        let mut sent = lock(&inner.emitted);
                        let from = sent.get(&m.id).copied().unwrap_or(0).max(*len);
                        if m.is_streaming {
                            sent.insert(m.id, from.max(m.body.len()));
                        } else {
                            sent.remove(&m.id);
                        }
                        from
                    };
                    if m.body.len() > from
                        && let Some(delta) = m.body.get(from..)
                    {
                        inner.emit(Event::MessageDelta {
                            workspace: wsid.clone(),
                            chat: ChatId::new(sid.to_string()),
                            message: MessageId::new(m.id.to_string()),
                            delta: delta.to_string(),
                        });
                    }
                    if *streaming && !m.is_streaming {
                        let error = m.body.strip_prefix("[error] ").map(|e| e.lines().next().unwrap_or_default().to_string());
                        inner.emit(Event::MessageDone {
                            workspace: wsid.clone(),
                            chat: ChatId::new(sid.to_string()),
                            message: MessageId::new(m.id.to_string()),
                            error,
                        });
                        if synced && m.actor_identity.as_ref().is_none_or(|a| a.id != me) {
                            notify_if_mentioned(inner, &s, m, &me);
                        }
                        inner.emit(Event::AgentsChanged { workspace: wsid.clone() });
                    }
                }
            }
        }
        if any_new && !fresh {
            inner.emit(Event::ChatsChanged { workspace: wsid.clone() });
        }
        if old.workflows != new.workflows {
            inner.emit(Event::WorkflowsChanged { workspace: wsid.clone() });
        }
        if !fresh && old_flow != new.flow {
            inner.emit(Event::MemoryChanged { workspace: wsid.clone() });
            inner.emit(Event::AgentsChanged { workspace: wsid.clone() });
        }
        for r in &s.workflow_runs {
            if old.runs.get(&r.id) != new.runs.get(&r.id) {
                inner.emit(Event::RunChanged { workspace: wsid.clone(), run: inner.run_view(&s, r) });
            }
        }
        if any_new {
            // A reply starting or ending changes who is busy.
            inner.emit(Event::AgentsChanged { workspace: wsid.clone() });
        }
        let hosts = inner.host_labels(ws);
        let hidden = inner.hidden_proposals(ws);
        for p in &s.proposals {
            if old.proposals.get(&p.id) != new.proposals.get(&p.id) {
                inner.emit(Event::ProposalChanged { workspace: wsid.clone(), proposal: views::proposal(&s, p, &me, &hosts, &hidden) });
            }
        }
    }
    triggers
}

fn notify_if_mentioned(inner: &Inner, s: &ChatSession, m: &ligus_core::ChatMessage, me: &str) {
    if !lock(&inner.local).notify_mentions {
        return;
    }
    // The desktop's rule (`message_mentions_me`): by id, by role, or by name.
    let mine = s.members.iter().find(|x| x.actor.id == me);
    let my_member = mine.map(|x| x.id.as_str());
    let my_name = mine.map(|x| x.actor.display_name.as_str()).unwrap_or("");
    let roles: Vec<_> = mine.map(|x| x.role).into_iter().collect();
    let named_me = ligus_runtime::message_mentions(m, s).reaches(|id| id == me || Some(id) == my_member, &roles, my_name);
    if !named_me {
        return;
    }
    let who = match views::message(s, m).author {
        Author::Member { name, .. } => name,
        a => a.display(),
    };
    let n = {
        let mut l = lock(&inner.local);
        let n = l.push_notification(
            &s.workspace_id.to_string(),
            Some(&s.id.to_string()),
            format!("{who} mentioned you in {}", s.title),
            views::ms(&m.created_at),
        );
        let _ = l.save(&inner.cfg.data_dir);
        n
    };
    inner.emit(Event::Notification(Notification {
        id: NotificationId::new(n.id),
        workspace: WorkspaceId::new(n.workspace),
        chat: n.chat.map(ChatId::new),
        text: n.text,
        at: n.at,
        read: false,
    }));
}

impl Inner {
    /// Start `ws`'s sync loop (when it has a relay) and its host loop.
    pub(crate) fn start_loops(self: &Arc<Self>, ws: Uuid) {
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        if let Some(old) = lock(&self.loops).insert(ws, stop_tx) {
            let _ = old.send(true);
        }
        let Ok(saved) = self.saved(ws) else { return };
        let wsid = WorkspaceId::new(ws.to_string());
        if saved.relay().is_some() {
            let built = (|| -> crate::backend::Result<SyncLoop> {
                let engine = self.sync_engine(&saved)?;
                let store = ligus_runtime::EventStore::open(self.db()).map_err(super::one_line)?;
                let feed = self.core()?.feed.clone();
                let relay = ligus_runtime::RelayClient::new(saved.relay().unwrap_or_default()).with_auth(saved.secrets.token.clone());
                Ok(SyncLoop { engine, store, feed, workspace_id: ws, every: self.cfg.sync_every, live: Some((relay, saved.room.clone())) })
            })();
            match built {
                Ok(lp) => {
                    let me = self.clone();
                    let w = wsid.clone();
                    let rx = stop_rx.clone();
                    let mut joined = false;
                    // Box admissions to close: after anything arrived, and
                    // every minute besides.
                    let (mut admissions_due, mut admissions_at) = (true, std::time::Instant::now());
                    tokio::spawn(lp.run(rx, move |st| {
                        let status = match st {
                            SyncStatus::Synced(r) => {
                                // Another lt on this data dir may have linked
                                // it to the person's devices: follow it, even
                                // with nobody at the keyboard.
                                let me3 = me.clone();
                                tokio::task::spawn_blocking(move || me3.follow_disk());
                                // A joiner redeems its invite once the config log arrives.
                                if !joined && r.pulled > 0 {
                                    joined = true;
                                    let me2 = me.clone();
                                    tokio::task::spawn_blocking(move || me2.activate(ws));
                                }
                                // As an Admin's device (the desktop does the same),
                                // close the box admissions that settled (redeemed,
                                // or lapsed unredeemed), so a holder of the code
                                // can't slot a backdated redemption in later. Only
                                // when caught up (a round that pulled nothing): a
                                // closure naming no redemption would deny one still
                                // waiting on the relay. A no-op below Admin.
                                if r.pulled > 0 {
                                    admissions_due = true;
                                } else if admissions_due || admissions_at.elapsed() >= Duration::from_secs(60) {
                                    (admissions_due, admissions_at) = (false, std::time::Instant::now());
                                    let me2 = me.clone();
                                    tokio::task::spawn_blocking(move || {
                                        if let Err(e) = me2.write(ws, |svc| svc.close_settled_admissions(ws)) {
                                            me2.emit(Event::Sync {
                                                workspace: WorkspaceId::new(ws.to_string()),
                                                status: format!("closing settled box admissions: {}", e.0),
                                            });
                                        }
                                    });
                                }
                                "synced".to_string()
                            }
                            SyncStatus::Failed(e) => format!("sync failed: {e}"),
                        };
                        me.emit(Event::Sync { workspace: w.clone(), status });
                    }));
                }
                Err(e) => self.emit(Event::Sync { workspace: wsid.clone(), status: format!("sync off: {}", e.0) }),
            }
        }
        if let Some(url) = saved.relay() {
            let relay = ligus_runtime::RelayClient::new(url).with_auth(saved.secrets.token.clone());
            tokio::spawn(presence_loop(self.clone(), saved.clone(), relay, stop_rx.clone()));
        }
        let me = self.clone();
        let mut stop = stop_rx;
        tokio::spawn(async move {
            let mut last_touch = std::time::Instant::now();
            loop {
                tokio::select! {
                    _ = stop.changed() => return,
                    _ = tokio::time::sleep(me.cfg.sync_every) => {}
                }
                if *stop.borrow() {
                    return;
                }
                let i = me.clone();
                let touch = last_touch.elapsed() >= Duration::from_secs(60);
                if touch {
                    last_touch = std::time::Instant::now();
                }
                let _ = tokio::task::spawn_blocking(move || host_tick(&i, ws, touch)).await;
            }
        });
    }
}

/// The relay's presence channel for `saved`: publish this host's beat and the
/// leases of the runs it drives, read every host's beat. A changed online
/// set reloads the host and agent views.
async fn presence_loop(
    inner: Arc<Inner>,
    saved: ligus_runtime::connection_settings::SavedWorkspace,
    relay: ligus_runtime::RelayClient,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    use ligus_core::lease::{DEFAULT_LEASE_MS, LeaseScope};
    use ligus_runtime::host_presence::{fetch_host_beats, publish_host_beat};
    use ligus_runtime::lease_presence::{LeaseBeat, publish_lease_beat};
    let ws = saved.id();
    let wsid = WorkspaceId::new(ws.to_string());
    let room = saved.room.clone();
    let mut online: Vec<String> = Vec::new();
    loop {
        let i = inner.clone();
        let w = saved.clone();
        let prepared = tokio::task::spawn_blocking(move || -> Option<_> {
            let core = i.core().ok()?;
            let device = core.identity.device.id;
            let src = i.key_source(&w).ok()?;
            let store = ligus_runtime::EventStore::open(i.db()).ok()?;
            let ring = ligus_runtime::keyring::KeyringEngine::from_store(&store, ws, &src);
            let now_ms = ligus_core::flow::now_ms();
            // The leases of the runs this device drives, signed by it.
            let leases: Vec<LeaseBeat> = {
                let mut drivers = lock(&i.drivers);
                let reader = lock(&core.reader);
                drivers
                    .iter_mut()
                    .filter(|(_, d)| d.ws == ws)
                    .map(|(run, d)| {
                        d.beat += 1;
                        reader.sign_lease_beat(LeaseBeat {
                            scope: LeaseScope::WorkflowRun,
                            scope_id: run.to_string(),
                            holder: device,
                            epoch: d.epoch,
                            beat: d.beat,
                            lease_ms: DEFAULT_LEASE_MS,
                            at_ms: now_ms,
                            signature: Vec::new(),
                        })
                    })
                    .collect()
            };
            Some((device, ring, leases))
        })
        .await
        .ok()
        .flatten();
        if let Some((device, ring, leases)) = prepared {
            let device = device.to_string();
            inner.beats(ws).record(&device, ligus_core::Timestamp::now().inner().unix_timestamp());
            let _ = publish_host_beat(&relay, &room, &ring, &device).await;
            for beat in &leases {
                let _ = publish_lease_beat(&relay, &room, &ring, beat).await;
            }
            if let Ok(beats) = fetch_host_beats(&relay, &room, &ring).await {
                inner.beats(ws).merge(beats);
            }
            let i = inner.clone();
            let now_online = tokio::task::spawn_blocking(move || {
                let now = ligus_core::Timestamp::now();
                let mut ids: Vec<String> = i
                    .read(ws, |svc| svc.list_hosts(ws))
                    .unwrap_or_default()
                    .iter()
                    .filter(|h| i.beats(ws).is_online(h, &now))
                    .map(|h| h.id.clone())
                    .collect();
                ids.sort();
                ids
            })
            .await
            .unwrap_or_default();
            if now_online != online {
                online = now_online;
                inner.emit(Event::HostsChanged { workspace: wsid.clone() });
                inner.emit(Event::AgentsChanged { workspace: wsid.clone() });
            }
        }
        tokio::select! {
            _ = stop.changed() => {}
            _ = tokio::time::sleep(inner.cfg.beat_every) => {}
        }
        if *stop.borrow() {
            return;
        }
    }
}

/// Heartbeat (every minute) and apply what was asked of this host.
fn host_tick(inner: &Inner, ws: Uuid, touch: bool) {
    let Ok(core) = inner.core() else { return };
    if inner.may_sign(&core, ws).is_err() {
        return;
    }
    let device = core.identity.device.id.to_string();
    if touch {
        let _ = inner.write(ws, |svc| svc.touch_host(ws, &device, 60));
    }
    let checkout = lock(&inner.local).checkouts.get(&ws.to_string()).cloned();
    let Some(repo) = checkout else { return };
    let repo = std::path::PathBuf::from(repo);
    // The tick pins `ws`'s founder under the same lock as each decision:
    // another workspace's write may re-pin the shared service in between.
    let founder = inner.founder(ws);
    let tick = ligus_runtime::sync_service::apply_tick(&core.svc, ws, founder.as_deref(), &|_| Some(repo.clone()));
    if let Some(e) = tick.listing_error {
        inner.emit(Event::Sync { workspace: WorkspaceId::new(ws.to_string()), status: format!("apply: {e}") });
    }
}
