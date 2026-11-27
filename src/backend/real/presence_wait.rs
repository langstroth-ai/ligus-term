//! The passkey hand-off on a real lt device: what a setup run offers its
//! writes ([`Inner::handoff_ctx`]), and the hand-offs left waiting in the
//! data dir's `presence-pending.json` when lt quit, offered on the next
//! start: listed, waited for again (then written with the passkey's
//! assertion, as `ligus handoff wait` does), or cancelled.
//!
//! Everything here reads with the read service: the hand-off itself runs
//! inside a write that holds the write service.

use std::collections::HashSet;

use ligus_runtime::approval_mailbox::{PresencePending, PresencePendingStore, PresenceRequester};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::term_presence::{HandoffCtx, device_signer, mailbox_client};
use super::{Inner, one_line};
use crate::backend::model::{PendingPresence, WorkspaceId};
use crate::backend::{BackendError, Result, err};

/// The same relay, whatever its trailing slash.
fn same_relay(a: &str, b: &str) -> bool {
    a.trim().trim_end_matches('/') == b.trim().trim_end_matches('/')
}

impl Inner {
    fn now_ms(&self) -> i64 {
        match &self.cfg.mailbox_clock {
            Some(c) => c(),
            None => ligus_core::flow::now_ms(),
        }
    }

    fn pending_store(&self) -> PresencePendingStore {
        PresencePendingStore::new(&self.cfg.data_dir)
    }

    /// What a setup run in `ws` offers its writes: the passkey hand-off to
    /// the person's own desktop or phone, or `None` where it can't run (no
    /// relay, no account, no workspace key epochs, no other device of
    /// yours to ask): today's sheet then.
    pub(crate) fn handoff_ctx(&self, ws: Uuid) -> Option<HandoffCtx> {
        let saved = self.saved(ws).ok()?;
        let relay = saved.relay()?.to_string();
        let core = self.core().ok()?;
        core.author.account_id?;
        let founder_pin = self.read(ws, |svc| svc.founder_pin_hex(ws)).ok().flatten()?;
        let src = self.key_source(&saved).ok()?;
        let epochs = self.read(ws, move |svc| svc.approval_epoch_keys(ws, &src)).ok()?;
        if epochs.is_empty() {
            return None;
        }
        let me = core.identity.device.id;
        let mine: HashSet<Uuid> = self.read(ws, |svc| svc.my_device_ids(ws)).ok()?.into_iter().collect();
        if !mine.iter().any(|d| *d != me) {
            return None;
        }
        let device_seed = Zeroizing::new(self.identity_store().device_keypair(me).ok()?.seed_bytes());
        Some(HandoffCtx {
            data_dir: self.cfg.data_dir.clone(),
            ws,
            relay,
            token: saved.secrets.token.clone(),
            founder_pin,
            epochs,
            mine,
            device_seed,
            label: self.label(),
            clock: self.cfg.mailbox_clock.clone(),
        })
    }

    /// The live entries this device asked, in workspaces it has, that this
    /// process isn't waiting on already.
    fn pending_entries(&self) -> Result<Vec<PresencePending>> {
        let core = self.core()?;
        let me = core.identity.device.id;
        let live = self.pending_store().live(self.now_ms()).map_err(one_line)?;
        Ok(live
            .into_iter()
            .filter(|e| self.saved(e.workspace_id).is_ok() && !self.presence.is_live(&e.rid))
            .filter(|e| ligus_runtime::approval_mailbox::PresenceAsk::decode(&e.ask).is_ok_and(|a| a.device_id() == me))
            .collect())
    }

    pub(crate) fn pending_presence(&self) -> Result<Vec<PendingPresence>> {
        Ok(self
            .pending_entries()?
            .into_iter()
            .map(|e| PendingPresence {
                rid: e.rid,
                workspace: WorkspaceId::new(e.workspace_id),
                summary: e.summary,
                code: e.code,
                created_at: e.created_at,
            })
            .collect())
    }

    /// Resume entry `rid`: checked against this device (the draft's
    /// signer), its workspace and that workspace's relay.
    fn resume_entry(&self, rid: &str) -> Result<(PresencePending, PresenceRequester)> {
        let e = self
            .pending_entries()?
            .into_iter()
            .find(|e| e.rid == rid.trim())
            .ok_or_else(|| BackendError("nothing is waiting with that id here (a request expires after 15 minutes)".into()))?;
        let saved = self.saved(e.workspace_id)?;
        if !saved.relay().is_some_and(|r| same_relay(r, &e.relay)) {
            return err("that request went through another relay than the workspace's: cancel it and run the step again");
        }
        let core = self.core()?;
        self.may_sign(&core, e.workspace_id)?;
        let kp = self.identity_store().device_keypair(core.identity.device.id).map_err(one_line)?;
        let signer = device_signer(&kp.seed_bytes());
        let client = mailbox_client(&e.relay, saved.secrets.token.clone(), self.cfg.mailbox_clock.clone());
        let req = PresenceRequester::resume(client, &e, signer, Some(&self.cfg.data_dir)).map_err(one_line)?;
        Ok((e, req))
    }

    /// Wait for entry `rid` again; once the person's own device confirms
    /// it, write the drafted change with the passkey's assertion. Its
    /// summary, when written. Blocking.
    pub(crate) fn resume_presence(&self, rid: &str) -> Result<String> {
        let (e, req) = self.resume_entry(rid)?;
        let ws = e.workspace_id;
        let draft = req.draft().clone();
        if draft.workspace_id != ws {
            return err("that request is for another workspace than it says");
        }
        // The draft must be this device's own (its signer), stamped with the
        // account this device acts for; its challenge and words from it.
        let pd = self.read(ws, move |svc| svc.presence_draft_of(draft))?;
        if pd.env.actor_stamp.as_ref().and_then(|s| s.actor.account_id) != self.core()?.author.account_id {
            return err("that request isn't stamped with your account");
        }
        let mine: HashSet<Uuid> = self.read(ws, |svc| svc.my_device_ids(ws))?.into_iter().collect();
        let summary = pd.request.summary.clone();
        let assertion = self.presence.wait_resumed(&pd.request, req, &mine).map_err(BackendError)?;
        self.write(ws, move |svc| svc.commit_presence(pd, assertion))
            .map_err(|e2| BackendError(format!("confirmed, but it couldn't be written ({}): run the step again", e2.0)))?;
        Ok(summary)
    }

    /// End entry `rid` (the cancel abort; the other device drops it). A
    /// damaged entry, or one another device asked, is only forgotten.
    pub(crate) fn cancel_pending_presence(&self, rid: &str) -> Result<()> {
        if self.presence.is_live(rid) {
            return err("lt is waiting for that one: cancel it on its sheet");
        }
        match self.resume_entry(rid) {
            Ok((_, mut req)) => {
                let rt = tokio::runtime::Handle::current();
                rt.block_on(async {
                    let _ = req.cancel().await;
                    req.settle().await
                })
                .map_err(one_line)
            }
            Err(_) => self.pending_store().remove(rid.trim()).map_err(one_line),
        }
    }
}
