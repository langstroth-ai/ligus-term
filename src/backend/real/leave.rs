//! Member exits and this computer's synced name (Ligus's workspace model
//! v3, 10.1 and 10.4).
//!
//! - **Leave workspace.** lt writes its person's own `MemberLeft` (with
//!   their passkey when the account holds one), pushes it, then stops
//!   syncing the workspace and forgets it here. Others see the person as
//!   "asked to leave" until an Admin removes them. Leaving revokes nothing.
//!   A linked computer can't make its person leave: Ligus refuses it, and
//!   lt says so.
//! - **Following a rename.** When the person renames this computer on their
//!   desktop, the synced name (set by another device, later than the name lt
//!   keeps) becomes the kept name, live, and the other workspaces get it
//!   too. lt's own renames are never followed back.

use std::sync::Arc;

use ligus_core::ChatSession;
use ligus_runtime::chat_service::{LEAVE_CONSEQUENCE, LeaveCheck, NOT_REVOKING};
use uuid::Uuid;

use super::{Inner, SECRET_SCOPE, label, lock, one_line};
use crate::backend::model::{LeaveState, LeaveView};
use crate::backend::{Event, Result};

pub(crate) fn view(check: LeaveCheck) -> LeaveView {
    let (state, refusal) = match check {
        LeaveCheck::Ready => (LeaveState::Ready, None),
        LeaveCheck::NeedsPasskey => (LeaveState::Passkey, None),
        LeaveCheck::AlreadyLeft => (LeaveState::Left, None),
        LeaveCheck::Refused(why) => (LeaveState::Refused, Some(ligus_runtime::display::shown(&why))),
    };
    LeaveView { state, refusal, line: LEAVE_CONSEQUENCE.into(), reminder: NOT_REVOKING.into() }
}

impl Inner {
    pub(crate) fn leave_check(&self, ws: Uuid) -> Result<LeaveView> {
        self.write(ws, |svc| svc.leave_check(ws)).map(view)
    }

    /// Write the leave (or nothing, when it was written before), then push
    /// it. The caller forgets the workspace once this returns.
    pub(crate) fn write_leave(&self, ws: Uuid) -> Result<()> {
        self.write(ws, |svc| svc.leave_workspace(ws)).map(|_| ())
    }

    /// Push this workspace's unsent events (the leave) to its relay.
    pub(crate) async fn push_now(self: &Arc<Self>, ws: Uuid) -> Result<()> {
        let saved = self.saved(ws)?;
        if saved.relay().is_none() {
            return Ok(());
        }
        let mut engine = self.sync_engine(&saved)?;
        let db = self.db();
        tokio::task::spawn_blocking(move || {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async move {
                let store = ligus_runtime::EventStore::open(db).map_err(one_line)?;
                engine.push_new(&store).await.map(|_| ()).map_err(one_line)
            })
        })
        .await
        .map_err(one_line)?
    }

    /// Stop syncing `ws` and forget it on this device.
    pub(crate) fn forget(&self, ws: Uuid) -> Result<()> {
        if let Some(stop) = lock(&self.loops).remove(&ws) {
            let _ = stop.send(true);
        }
        lock(&self.list).remove(ws, self.secrets.as_ref(), SECRET_SCOPE).map_err(one_line)?;
        self.save_list()?;
        if let Ok(core) = self.core() {
            let _ = lock(&core.svc).store().forget_workspace(ws);
        }
        self.emit(Event::WorkspacesChanged);
        Ok(())
    }

    /// Follow a rename of this computer made on its person's desktop: in
    /// `config` (`ws`'s config log), a synced name for this device set by
    /// another device after lt kept its own name. Returns the new name.
    pub(crate) fn follow_synced_label(&self, ws: Uuid, config: &ChatSession) -> Option<String> {
        let core = self.core().ok()?;
        let account = core.author.account_id?;
        let me = core.identity.device.id;
        let name = config.device_labels.get(&me)?.get(&account)?.clone();
        let set = config.device_label_set.get(&(me, account))?;
        if set.by == Some(me) || name.trim().is_empty() || name == self.label() {
            return None;
        }
        let kept = label::stored_at(&self.cfg.data_dir);
        let renamed = std::time::UNIX_EPOCH + std::time::Duration::from_nanos(set.at.inner().unix_timestamp_nanos().max(0) as u64);
        if kept.is_some_and(|k| k >= renamed) {
            return None;
        }
        self.keep_label(&name);
        let name = self.label();
        // The other workspaces get the same name, written by this device.
        let others: Vec<Uuid> = lock(&self.list).workspaces.iter().map(|w| w.id()).filter(|w| *w != ws).collect();
        for w in others {
            let _ = self.write(w, |svc| {
                let d = svc.device_id();
                svc.set_device_label(w, d, &name)
            });
        }
        self.emit(Event::DeviceRenamed { name: name.clone() });
        Some(name)
    }
}
