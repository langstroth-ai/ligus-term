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

/// What a write says when this lt's open identity is no longer the data
/// dir's (another lt on it linked it, and following it failed).
pub(crate) const STALE: &str = "this lt is out of date: another lt on this data dir linked this computer to your devices since it opened. \
     Quit this lt and start it again (it then acts as the person it was linked to)";

/// What a write says while a link is being redeemed on this install.
pub(crate) const LINKING: &str = "this computer is being linked to your devices: try again in a moment";

/// What a write says in a workspace a linked install wasn't linked into.
pub(crate) const NOT_LINKED_HERE: &str = "this computer is linked to someone's devices in another workspace, not this one: \
     add it to this workspace from the desktop (My devices > Add a computer), or use another LT_DATA_DIR";

/// What a refused write says when the workspace certifies this device for
/// `owner` but this install doesn't act as them.
pub(crate) fn mismatch(owner: &str) -> String {
    format!(
        "Refused: this computer is certified as one of {}'s devices in this workspace, but lt is signing as its old member, \
         who isn't a member here any more (its link record, computer-link.json, is missing or names someone else). \
         Fix: re-link this computer: palette > Link this computer to my devices (or lt --plain link-request), \
         approve it on the desktop, then paste the code back",
        shown(owner)
    )
}

/// A refusal of the role gate (`role Viewer cannot perform messageAppended`),
/// in words: what was refused, why, and the fix. A device that signs as
/// someone who isn't a member (removed, or an install still signing as its
/// old member) is told to link this computer again; a role too low, which
/// role it needs. Anything else passes through.
pub(crate) fn explain(svc: &ligus_runtime::ChatService, ws: Uuid, e: BackendError) -> BackendError {
    let s = &e.0;
    let action = if let Some(rest) = s.strip_prefix("role ") {
        let Some((_, kind)) = rest.split_once(" cannot perform ") else { return e };
        match kind.trim() {
            "messageAppended" => "posting a message".to_string(),
            "sessionCreated" => "starting a chat".to_string(),
            "proposalVoteCast" => "voting".to_string(),
            other => format!("this change ({other})"),
        }
    } else if let Some(rest) = s.strip_prefix("Refused: this device signs as ") {
        // The runtime's own explanation of a non-member's refusal: lt says
        // what to do on this computer instead.
        match rest.split_once(", so ").and_then(|(_, a)| a.split_once(" isn't allowed")) {
            Some((action, _)) if rest.contains("isn't a member of this workspace") => action.to_string(),
            _ => return e,
        }
    } else {
        return e;
    };
    let me = svc.author().clone();
    let config = svc.load_in(ws, ligus_core::workspace_config_session_id(ws)).ok().flatten();
    let member = config.as_ref().and_then(|c| c.members.iter().find(|m| m.id == me.id).cloned());
    // Only the wording: a device certified for someone this install doesn't
    // act as (its link record missing or someone else's) is told to re-link.
    // Never a block: anyone can publish a certificate for a public key.
    let owner = match svc.device_standing(ws, Some(&me.id)) {
        Ok(ligus_runtime::device_enroll::Standing::OwnerDevice(o)) if o.account_id != me.account_id => Some(o),
        _ => None,
    };
    match (member, owner) {
        (None, Some(owner)) => BackendError(mismatch(&owner.display_name)),
        (None, None) => BackendError(format!(
            "Refused: this computer signs as {}, who isn't a member of this workspace any more, so {action} isn't allowed. \
             Why: an Admin removed that member (the duplicate left when this computer was linked to your devices?). \
             Fix: re-link this computer: palette > Link this computer to my devices (or lt --plain link-request), \
             approve it on your desktop under Settings > Profile > My devices > Add a computer, then paste the code back",
            shown(&me.display_name)
        )),
        (Some(m), _) => BackendError(format!(
            "Refused: your role here is {:?}, and {action} needs a higher one. Fix: ask an Owner or Admin to change your role (People)",
            m.role
        )),
    }
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

    /// Push what this device holds for `w` and the relay hasn't got.
    async fn push_unpushed(&self, w: &SavedWorkspace) -> Result<()> {
        let mut engine = self.sync_engine(w)?;
        let unpushed = engine.take_unpushed(&EventStore::open(self.db()).map_err(one_line)?).map_err(one_line)?;
        engine.push_envelopes(&unpushed).await.map_err(one_line)?;
        engine.take_unpushed(&EventStore::open(self.db()).map_err(one_line)?).map_err(one_line)?;
        Ok(())
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
        // The data dir's identity changes step by step below: nothing
        // follows it until the link is recorded (or rolled back).
        struct Linking<'a>(&'a std::sync::atomic::AtomicUsize);
        impl Drop for Linking<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        self.linking.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _linking = Linking(&self.linking);
        let (g, device, _) = self.open_grant(grant)?;
        let ws = g.workspace_id;
        let ids = self.identity_store();
        let old = ids.load().map_err(one_line)?.ok_or_else(|| BackendError("make a link request first".into()))?;
        let previous_entry = lock(&self.list).get(ws).cloned();
        // What this install wrote as its own member and hasn't pushed yet
        // goes out now, while it is still a member: pushed after the desktop
        // removes that member, it would be refused there.
        if let Some(w) = &previous_entry {
            let _ = self.push_unpushed(w).await;
        }
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
