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
//! - **Sync loop.** `hive_runtime::sync_service::SyncLoop` per workspace
//!   with a relay.
//! - **Host loop.** Heartbeats this device as a host and runs the
//!   apply-on-host loop (`hive_runtime::sync_service::apply_tick`) when a
//!   checkout is bound.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use hive_core::{ChatSession, MessageRole, workspace_config_session_id};
use hive_runtime::chat_service::{ChangeOrigin, ChatChange};
use hive_runtime::sync_service::{SyncLoop, SyncStatus};
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
    }
}

fn proposal_key(s: &ChatSession, p: &hive_core::ActionProposal) -> String {
    let requests = s.apply_requests.values().filter(|r| r.proposal_id == p.id).count();
    let outcomes = s.apply_requests.values().filter(|r| r.proposal_id == p.id && s.apply_outcomes.contains_key(&r.id)).count();
    format!("{:?}/{}/{}/{}/{}", p.status, p.approvals.len(), requests, outcomes, p.dismissed)
}

/// Remember every session as it is now, so only later changes are events.
pub fn prime(inner: &Inner, core: &Core) {
    let r = lock(&core.reader);
    let Ok(ids) = r.store().list_session_ids() else { return };
    let mut snaps = lock(&inner.snaps);
    for sid in ids {
        if let Ok(Some(s)) = r.load(sid) {
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
    let me = core.identity.actor().id;
    let my_device = core.identity.device.id;
    let known: HashSet<Uuid> = lock(&inner.list).workspaces.iter().map(|w| w.id()).collect();
    for (sid, (ws, synced)) in batch {
        if !known.contains(&ws) {
            continue;
        }
        let Ok(Some(s)) = inner.read(ws, |svc| svc.load(sid)) else { continue };
        let wsid = WorkspaceId::new(ws.to_string());
        let old = lock(&inner.snaps).0.get(&sid).cloned();
        let new = snap_of(&s);
        lock(&inner.snaps).0.insert(sid, new.clone());
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
                    }
                }
            }
        }
        if any_new && !fresh {
            inner.emit(Event::ChatsChanged { workspace: wsid.clone() });
        }
        let hosts = inner.host_labels(ws);
        for p in &s.proposals {
            if old.proposals.get(&p.id) != new.proposals.get(&p.id) {
                inner.emit(Event::ProposalChanged { workspace: wsid.clone(), proposal: views::proposal(&s, p, &me, &hosts) });
            }
        }
    }
    triggers
}

fn notify_if_mentioned(inner: &Inner, s: &ChatSession, m: &hive_core::ChatMessage, me: &str) {
    if !lock(&inner.local).notify_mentions {
        return;
    }
    // The desktop's rule (`message_mentions_me`): by id, by role, or by name.
    let mine = s.members.iter().find(|x| x.actor.id == me);
    let my_member = mine.map(|x| x.id.as_str());
    let my_name = mine.map(|x| x.actor.display_name.as_str()).unwrap_or("");
    let roles: Vec<_> = mine.map(|x| x.role).into_iter().collect();
    let named_me = hive_runtime::message_mentions(m, s).reaches(|id| id == me || Some(id) == my_member, &roles, my_name);
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
                let store = hive_runtime::EventStore::open(self.db()).map_err(super::one_line)?;
                let feed = lock(&self.core()?.svc).change_feed();
                let relay = hive_runtime::RelayClient::new(saved.relay().unwrap_or_default()).with_auth(saved.secrets.token.clone());
                Ok(SyncLoop { engine, store, feed, workspace_id: ws, every: self.cfg.sync_every, live: Some((relay, saved.room.clone())) })
            })();
            match built {
                Ok(lp) => {
                    let me = self.clone();
                    let w = wsid.clone();
                    let rx = stop_rx.clone();
                    let mut joined = false;
                    tokio::spawn(lp.run(rx, move |st| {
                        let status = match st {
                            SyncStatus::Synced(r) => {
                                // A joiner redeems its invite once the config log arrives.
                                if !joined && r.pulled > 0 {
                                    joined = true;
                                    let me2 = me.clone();
                                    tokio::task::spawn_blocking(move || me2.activate(ws));
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

/// Heartbeat (every minute) and apply what was asked of this host.
fn host_tick(inner: &Inner, ws: Uuid, touch: bool) {
    let Ok(core) = inner.core() else { return };
    let device = core.identity.device.id.to_string();
    if touch {
        let _ = inner.write(ws, |svc| svc.touch_host(ws, &device, 60));
    }
    let checkout = lock(&inner.local).checkouts.get(&ws.to_string()).cloned();
    let Some(repo) = checkout else { return };
    let repo = std::path::PathBuf::from(repo);
    let founder = inner.founder(ws);
    lock(&core.svc).set_trusted_founder(founder);
    let tick = hive_runtime::sync_service::apply_tick(&core.svc, ws, &|_| Some(repo.clone()));
    if let Some(e) = tick.listing_error {
        inner.emit(Event::Sync { workspace: WorkspaceId::new(ws.to_string()), status: format!("apply: {e}") });
    }
}
