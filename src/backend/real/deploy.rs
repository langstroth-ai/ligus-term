//! Connecting to a deployment: checking an invite before joining, founding
//! the workspace a `ligus-local` box or the Kubernetes chart was pointed at,
//! and admitting their boxes as service members.
//!
//! - **Invite check** (`inspect`): the desktop's rule. The invite's founder
//!   signature is verified (`WorkspaceConn::signed_by`, against the devices
//!   the shared logs revoked) and its founder key compared with what this
//!   device's workspaces register for that account
//!   (`dm::check_founder`). A contradicting key refuses the join.
//! - **Founding a deployment's workspace** (`connect`): a box never founds a
//!   workspace (`ligus serve` stops with "A box never founds a workspace").
//!   `lt` takes the deployment's relay, room and key (the chart's workspace
//!   Secret, or the `ligus-local` env file), checks the relay holds no copy
//!   yet, and founds it as `create_workspace` does: a founder passkey and a
//!   recovery key.
//! - **Box admission** (`admit`): `issue_box_admission` (Admin, with a
//!   passkey), then the same code `ligus admit issue` builds: a signed
//!   `hivews1:` invite carrying the redeem key, sealed to the box's
//!   `ligusjoin1:` request when there is one.

use ligus_core::workspace_config_session_id;
use ligus_runtime::box_admission::{self, JoinRequest};
use ligus_runtime::connection_settings::SavedWorkspace;
use ligus_runtime::dm::{self, FounderTrust};
use ligus_runtime::workspace_invite::{WorkspaceConn, decode_workspace_invite, usable_relay_url};
use ligus_runtime::{EventStore, directory_trust};
use uuid::Uuid;

use super::{Inner, lock, one_line};
use crate::backend::model::*;
use crate::backend::{BackendError, Result, err};

/// The refusal for a pasted direct-message invite.
pub const NO_DMS: &str = "that's a direct-message invite; lt doesn't do direct messages (open it in the Ligus app)";

/// The refusal for an invite whose founder key contradicts this device's logs.
pub const MISMATCH: &str = "the invite's founder key doesn't match the one your shared workspaces have for that person: not joining";

fn trust(t: FounderTrust) -> InviteTrust {
    match t {
        FounderTrust::KnownMatch => InviteTrust::Known,
        FounderTrust::Claimed => InviteTrust::Claimed,
        FounderTrust::Mismatch => InviteTrust::Mismatch,
        FounderTrust::Unknown => InviteTrust::Unknown,
        FounderTrust::NoFingerprint => InviteTrust::NoFingerprint,
    }
}

impl Inner {
    /// The people this device shares a workspace with (none before it has
    /// an identity).
    fn co_members(&self) -> Vec<directory_trust::CoMember> {
        let Ok(core) = self.core() else { return Vec::new() };
        let Ok(store) = EventStore::open(self.db()) else { return Vec::new() };
        directory_trust::co_members(&store, core.identity.account.id)
    }

    /// What `conn` says about who sent it, as the desktop labels an invite.
    pub(crate) fn invite_trust(&self, conn: &WorkspaceConn) -> (InviteTrust, Option<String>) {
        let people = self.co_members();
        let signer = conn.signed_by(&|d| people.iter().any(|p| p.revoked_devices.contains(&d)));
        let known = dm::known_fingerprints(&people);
        let t = dm::check_founder(conn.founder_actor_id.as_deref(), conn.fk.as_deref(), &known, signer);
        let founder = conn.founder_actor_id.as_deref().and_then(|f| Uuid::parse_str(f).ok());
        let name = founder.and_then(|f| people.iter().find(|p| p.account_id == f)).map(|p| p.display_name.clone());
        (trust(t), name)
    }

    pub(crate) fn inspect(&self, invite: &str) -> Result<InviteCheck> {
        let conn = decode_workspace_invite(invite).map_err(BackendError)?;
        if conn.dm_account.is_some() {
            return err(NO_DMS);
        }
        let (trust, founder_name) = self.invite_trust(&conn);
        let already_joined = lock(&self.list).get(conn.id()).is_some();
        Ok(InviteCheck {
            workspace_name: ligus_runtime::human_presence::clean_display_name(&conn.display_name()),
            relay_url: conn.relay_url.trim().to_string(),
            founder_name,
            safety_code: conn.fk.as_deref().and_then(directory_trust::safety_code),
            trust,
            already_joined,
        })
    }

    /// Admit a box: `request` is its `ligusjoin1:` code, or none for a
    /// bearer code.
    pub(crate) fn admit(&self, ws: Uuid, request: Option<&str>) -> Result<BoxAdmission> {
        let w = self.saved(ws)?;
        if w.relay().is_none() {
            return err("this workspace has no relay, so no box could reach it (start lt with LT_RELAY_URL)");
        }
        let me = self.me();
        let role = self.read(ws, |svc| Ok(svc.load(workspace_config_session_id(ws))?.map(|c| c.role_of(&me))))?;
        if role.is_none_or(|r| r.rank() < ligus_core::WorkspaceRole::Admin.rank()) {
            return err("only an Admin or Owner admits a box");
        }
        let request = request.map(str::trim).filter(|r| !r.is_empty()).map(JoinRequest::decode).transpose().map_err(BackendError)?;
        let ttl = if request.is_some() { box_admission::DEFAULT_TTL_MS } else { box_admission::DEFAULT_BEARER_TTL_MS };
        let core = self.core()?;
        let src = self.key_source(&w)?;
        let cert = core.identity.device.certificate.clone();
        let mut conn: WorkspaceConn = w.conn();
        conn.name = String::new();
        let seed = self.write(ws, |svc| {
            let (inv, seed) = svc.issue_box_admission(ws, ttl, request.as_ref())?;
            conn.inv = Some(inv.to_string());
            let ek = svc.delivered_epoch_keys(ws, &src)?;
            if !ek.is_empty() {
                conn.ek = ek;
            }
            let (name, ek_json) = (conn.display_name(), conn.ek_json());
            conn.sig = svc.sign_invite(&conn.sig_claims(&name, &ek_json), &cert);
            Ok(seed)
        })?;
        let code = ligus_runtime::workspace_invite::encode_workspace_invite(&conn);
        let code = box_admission::with_redeem_key(&code, &seed).map_err(BackendError)?;
        let expires_at = super::views::ms(&ligus_core::Timestamp::now()) + ttl;
        Ok(match &request {
            Some(r) => BoxAdmission {
                code: r.seal(&code).map_err(BackendError)?,
                sealed: true,
                fingerprint: Some(r.fingerprint()),
                label: Some(r.label.clone()).filter(|l| !l.is_empty()),
                expires_at,
            },
            None => BoxAdmission { code, sealed: false, fingerprint: None, label: None, expires_at },
        })
    }
}

/// Read a box's join request without admitting it: the fingerprint the
/// Admin compares with the box's terminal before the passkey prompt.
pub(crate) fn inspect_join_request(request: &str) -> Result<JoinRequestCheck> {
    let r = JoinRequest::decode(request.trim()).map_err(BackendError)?;
    Ok(JoinRequestCheck { fingerprint: r.fingerprint(), label: Some(r.label.clone()).filter(|l| !l.is_empty()) })
}

/// The connection a deployment spec describes (checked).
pub fn deployment_conn(spec: &DeploymentSpec, founder: String, fk: String) -> Result<WorkspaceConn> {
    let relay = spec.relay_url.trim();
    if !usable_relay_url(relay) {
        return err(format!(
            "{relay:?} isn't a relay address: give the deployment's relay URL, e.g. http://127.0.0.1:18443 after a kubectl port-forward"
        ));
    }
    let room = spec.room.trim();
    if room.is_empty() {
        return err("give the workspace room (LIGUS_WORKSPACE, or the chart's workspace Secret, key room)");
    }
    let key = spec.key.trim();
    if key.is_empty() {
        return err("give the workspace key (LIGUS_WORKSPACE_KEY_FILE, or the chart's workspace Secret, key key)");
    }
    let name = match spec.name.trim() {
        "" => room.to_string(),
        n => n.to_string(),
    };
    Ok(WorkspaceConn {
        name,
        relay_url: relay.to_string(),
        room: room.to_string(),
        key: Some(key.to_string()),
        icon: None,
        dm_account: None,
        founder_actor_id: Some(founder),
        fk: Some(fk),
        inv: None,
        ek: Vec::new(),
        sig: None,
    })
}

/// Whether the relay already holds `saved`'s workspace (its config log).
pub async fn relay_has_workspace(inner: &std::sync::Arc<Inner>, saved: &SavedWorkspace) -> Result<bool> {
    let ws = saved.id();
    let mut engine = {
        let i = inner.clone();
        let saved = saved.clone();
        tokio::task::spawn_blocking(move || i.sync_engine(&saved)).await.map_err(one_line)??
    };
    let mut store = EventStore::open(inner.db()).map_err(one_line)?;
    engine.catch_up(&mut store, 250).await.map_err(one_line)?;
    let config = workspace_config_session_id(ws);
    Ok(store.sessions_of_workspace(ws).map_err(one_line)?.contains(&config))
}
