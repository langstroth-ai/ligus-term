//! "Add to my devices": this install becomes one of a person's devices in a
//! workspace they are already in on their desktop, instead of a new member
//! (`ligus_runtime::computer_link`; the `ligus devices` CLI does the same).
//!
//! 1. [`Inner::link_request`]: the link request (no secret), remembered as
//!    pending in the data dir. An install that already holds a workspace
//!    asks with a new device key, kept next to the current one and switched
//!    to only on redeem, and proves (with its own account key) the member it
//!    was.
//! 2. The person approves it on their desktop with a passkey; the desktop
//!    shows a grant sealed to this computer.
//! 3. [`Inner::inspect_link_grant`]: who approved it, shown for the person
//!    to confirm (nothing saved).
//! 4. [`Inner::redeem_link`]: save the connection, pull, check the desktop's
//!    certificate for this device, record the link, and act as the person
//!    from then on (`ChatService::act_as_owner_device`, on every open).
//!
//! One person per install: a linked install refuses to found or join a
//! workspace, and a grant for anyone else.

use std::sync::Arc;

use ligus_runtime::computer_link::{Grant, LinkRequest, LinkState, OpenedGrant};
use ligus_runtime::connection_settings::SavedWorkspace;
use ligus_runtime::display::shown;
use ligus_runtime::{EventStore, StoredIdentity};
use uuid::Uuid;

use super::{Inner, lock, one_line};
use crate::backend::model::{LinkGrantView, LinkRequestView, WorkspaceSummary};
use crate::backend::{BackendError, Result, err};

fn now_ms() -> i64 {
    ligus_core::flow::now_ms()
}

/// The link state of `data_dir`; a damaged file is an error (it may name the
/// person this install acts for).
pub(crate) fn link_state(data_dir: &std::path::Path) -> Result<LinkState> {
    LinkState::load(data_dir)
        .map_err(|e| BackendError(format!("{} is unreadable ({e}): fix or remove it", LinkState::path(data_dir).display())))
}

/// What a linked install says when asked to found or join a workspace.
pub(crate) fn refusal(person: &str) -> String {
    crate::backend::fake::linked_refusal(&shown(person))
}

impl Inner {
    /// The person this install acts for, if it was linked.
    pub(crate) fn linked_person(&self) -> Option<ligus_core::ActorIdentity> {
        link_state(&self.cfg.data_dir).ok().and_then(|s| s.person)
    }

    /// Refuse founding or joining on a linked install.
    pub(crate) fn refuse_if_linked(&self) -> Result<()> {
        match self.linked_person() {
            Some(p) => err(refusal(&p.display_name)),
            None => Ok(()),
        }
    }

    pub(crate) fn link_request(self: &Arc<Self>) -> Result<LinkRequestView> {
        let ids = self.identity_store();
        let stored = match ids.load().map_err(one_line)? {
            Some(s) => s,
            // A fresh install: an identity named after this device.
            None => {
                let name = self.cfg.device_name.clone();
                ids.bootstrap(&name, "lt", &name).map_err(one_line)?
            }
        };
        if stored.service {
            return err("this data dir is a box (a service member): a box joins as its own member");
        }
        self.open_core(stored.clone())?;
        let mut state = link_state(&self.cfg.data_dir)?;
        let now = now_ms();
        // Already in use: ask with a new device key, so the key that signed
        // as this install's own member is never certified for the person.
        let fresh_key = !lock(&self.list).workspaces.is_empty();
        let (device_id, device_kp) = if fresh_key {
            match state.latest_pending(now).filter(|p| p.device_id != stored.device.id) {
                Some(p) => (p.device_id, ids.device_keypair(p.device_id).map_err(one_line)?),
                None => ids.prepare_device().map_err(one_line)?,
            }
        } else {
            (stored.device.id, ids.device_keypair(stored.device.id).map_err(one_line)?)
        };
        let ka = ids.key_agreement(device_id).map_err(one_line)?;
        let account_kp = ids.account_keypair(stored.account.id).map_err(one_line)?;
        let previous = fresh_key.then_some((stored.account.id, &account_kp));
        let request = LinkRequest::new(&device_kp, &ka.public_key_bytes(), &self.cfg.device_name, now, previous);
        state.add_pending(&request, now);
        state.save(&self.cfg.data_dir).map_err(one_line)?;
        Ok(LinkRequestView { code: request.encode(), fingerprint: request.fingerprint(), label: request.label, new_device: fresh_key })
    }

    /// Open `grant` with the key of the pending request it answers and check
    /// it offline: (the grant, the device it certifies, the person).
    fn open_grant(&self, grant: &str) -> Result<(OpenedGrant, Uuid, ligus_core::ActorIdentity)> {
        let ids = self.identity_store();
        let state = link_state(&self.cfg.data_dir)?;
        let mut opened = None;
        for p in &state.pending {
            let Ok(ka) = ids.key_agreement(p.device_id) else { continue };
            if let Ok(g) = Grant::open(grant, &ka) {
                opened = Some((g, p.device_id));
                break;
            }
        }
        let Some((g, device)) = opened else {
            if !grant.trim().starts_with(ligus_runtime::computer_link::GRANT_PREFIX) {
                return err("not a Ligus device grant (expected ligusgrant1:...): paste the code your desktop shows");
            }
            return err("this code doesn't answer a request made on this computer (or it was used already): make a new request");
        };
        let now = now_ms();
        if state.pending_for(&g.request, now).is_none() {
            return err("the request this code answers expired or was used: make a new request");
        }
        let key = ids.device_keypair(device).map_err(one_line)?.public_key_bytes();
        let person = g.check(&key, now).map_err(|e| BackendError(e.to_string()))?;
        if let Some(p) = &state.person
            && p.account_id != person.account_id
        {
            return err(format!("this computer is already one of {}'s devices: it can't be someone else's too", shown(&p.display_name)));
        }
        let others: Vec<String> = lock(&self.list).workspaces.iter().filter(|w| w.id() != g.workspace_id).map(|w| w.name.clone()).collect();
        if !others.is_empty() && state.person.is_none() {
            return err(format!(
                "this install also holds {}: link a computer from a fresh install (another LT_DATA_DIR), or leave those first",
                others.join(", ")
            ));
        }
        Ok((g, device, person))
    }

    pub(crate) fn inspect_link_grant(&self, grant: &str) -> Result<LinkGrantView> {
        let (g, _, person) = self.open_grant(grant)?;
        Ok(LinkGrantView {
            person_name: shown(&person.display_name),
            workspace_name: g.conn.display_name(),
            approver_fingerprint: g.approver_fingerprint(),
        })
    }

    /// Replace the open services with ones for `stored` (a switched device
    /// key, or a link just recorded).
    fn reopen_core(self: &Arc<Self>, stored: StoredIdentity) -> Result<()> {
        *self.core.write().unwrap_or_else(|e| e.into_inner()) = None;
        self.open_core(stored).map(|_| ())
    }

    /// Everything up to the link record. Writes nothing to the workspace.
    async fn redeem_steps(self: &Arc<Self>, g: &OpenedGrant) -> Result<ligus_core::ActorIdentity> {
        let ws = g.workspace_id;
        let (founder, fk) = g.conn.founder_pin().ok_or_else(|| BackendError("the code names no founder".into()))?;
        let mut conn = g.conn.clone();
        conn.sig = None;
        conn.inv = None;
        let saved = SavedWorkspace::from_conn(&conn, None);
        {
            let store = EventStore::open(self.db()).map_err(one_line)?;
            store.set_workspace_pin(ws, &founder, &fk).map_err(one_line)?;
        }
        lock(&self.list).upsert(saved.clone());
        self.save_list()?;
        let mut engine = self.sync_engine(&saved)?;
        let mut store = EventStore::open(self.db()).map_err(one_line)?;
        engine.catch_up(&mut store, 250).await.map_err(one_line)?;
        let now = now_ms();
        let g2 = g.clone();
        self.read(ws, move |svc| svc.check_computer_link(&g2, now))
    }

    pub(crate) async fn redeem_link(self: &Arc<Self>, grant: &str) -> Result<WorkspaceSummary> {
        let (g, device, _) = self.open_grant(grant)?;
        let ws = g.workspace_id;
        let ids = self.identity_store();
        let old = ids.load().map_err(one_line)?.ok_or_else(|| BackendError("make a link request first".into()))?;
        let previous_entry = lock(&self.list).get(ws).cloned();
        // The sync loop of a workspace this install held as its own member
        // stops: it is replaced.
        if let Some(stop) = lock(&self.loops).remove(&ws) {
            let _ = stop.send(true);
        }
        let switched = device != old.device.id;
        if switched {
            let stored = ids.adopt_device(device).map_err(one_line)?;
            self.reopen_core(stored)?;
        }
        let checked = self.redeem_steps(&g).await;
        let person = match checked {
            Ok(p) => p,
            Err(e) => {
                // Back to how it was: the old device, the old entry.
                {
                    let mut list = lock(&self.list);
                    match previous_entry {
                        Some(w) => list.upsert(w),
                        None => {
                            let _ = list.remove(ws, self.secrets.as_ref(), super::SECRET_SCOPE);
                        }
                    }
                }
                let _ = self.save_list();
                if switched {
                    let back = ids.adopt_device(old.device.id).map_err(one_line)?;
                    self.reopen_core(back)?;
                }
                return Err(e);
            }
        };
        let mut state = link_state(&self.cfg.data_dir)?;
        state.record_link(&person, ws).map_err(|e| BackendError(e.to_string()))?;
        state.save(&self.cfg.data_dir).map_err(one_line)?;
        // Open as the person from now on.
        let stored = ids.load().map_err(one_line)?.ok_or_else(|| BackendError("no identity".into()))?;
        self.reopen_core(stored)?;
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.activate(ws)).await.map_err(one_line)?;
        self.start_loops(ws);
        self.emit(crate::backend::Event::WorkspacesChanged);
        Ok(self.summary(&self.saved(ws)?))
    }
}
