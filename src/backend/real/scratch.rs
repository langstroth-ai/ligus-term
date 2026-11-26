//! Scratch repositories on lt (`ligus_runtime::repos::scratch_host`).
//!
//! lt is a person's own device, so it makes a chat's scratch repository
//! itself: an empty repository (one empty commit on `main`) under its
//! scratch root (`<data dir>/scratch/<slug>-<short id>` by default),
//! registered as a workspace repository flagged scratch (an Admin's, as
//! adding any repository), bound here and set as the chat's repository.
//! lt runs the chat's agents on this device, in the chat's own branch and
//! worktree of it, with proposals, approvals and the sandbox as for any
//! repository.
//!
//! The cleanup ([`sweep`]) runs from the host loop: a notice in the chats
//! two days before, then the removal of a scratch repository nobody kept
//! or worked in. It deletes only folders this device made and recorded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ligus_runtime::ChatService;
use ligus_runtime::repos::{scratch, scratch_host};
use uuid::Uuid;

use super::{Inner, one_line};
use crate::backend::model::*;
use crate::backend::{BackendError, Result, err};

/// How often the cleanup runs while lt is open (it also runs at start).
pub(crate) const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// The folders a scratch root or a kept scratch repository may never be
/// in: the desktop app's data dirs (as `ligus setup` lists them).
pub(crate) fn forbidden() -> Vec<PathBuf> {
    let id = crate::paths::DESKTOP_APP_ID;
    let mut v = vec![];
    if let Some(home) = dirs::home_dir() {
        v.push(home.join("Library/Application Support").join(id));
        v.push(home.join(".local/share").join(id));
    }
    // The platform's own (XDG_DATA_HOME, APPDATA, …).
    if let Some(d) = dirs::data_dir().map(|d| d.join(id))
        && !v.contains(&d)
    {
        v.push(d);
    }
    v
}

impl Inner {
    /// `<data dir>/scratch`: where this device's scratch repositories go
    /// unless its scratch settings name another root.
    pub(crate) fn scratch_root(&self) -> PathBuf {
        self.cfg.data_dir.join("scratch")
    }

    pub(crate) fn scratch_settings(&self, svc: &ChatService) -> scratch::Settings {
        scratch::settings(svc.store(), &self.scratch_root())
    }
}

/// The workspace's live repositories as a chat shows them, by id: the
/// scratch tag, and for a scratch repository this device made, its
/// cleanup day and whether the notice is due.
pub(crate) fn repo_views(i: &Inner, svc: &ChatService, ws: Uuid) -> HashMap<Uuid, ChatRepo> {
    let Ok(roster) = svc.workspace_roster(ws) else { return HashMap::new() };
    let settings = i.scratch_settings(svc);
    let mine = scratch::live(svc.store(), ws);
    let now = scratch::now();
    ligus_runtime::repos::live_repos(&roster)
        .into_iter()
        .map(|r| {
            let entry = mine.iter().find(|e| e.repo_id == Some(r.id));
            let x = entry.and_then(|e| scratch::expiry(e, &settings, now));
            let view = ChatRepo {
                id: r.id.to_string(),
                name: r.display_name.clone(),
                scratch: r.is_scratch() || entry.is_some(),
                mine: entry.is_some(),
                removes_on: x.map(|x| scratch::date(x.remove_at)),
                notice: x.is_some_and(|x| x.notice),
                keep_to: entry.map(|e| keep_suggestion(&e.name)),
            };
            (r.id, view)
        })
        .collect()
}

/// Where "Keep it as a repo" suggests: `~/projects/<slug>` when there is a
/// `~/projects`, else `~/<slug>`.
fn keep_suggestion(name: &str) -> String {
    let slug = scratch::slug(name);
    let projects = dirs::home_dir().is_some_and(|h| h.join("projects").is_dir());
    if projects { format!("~/projects/{slug}") } else { format!("~/{slug}") }
}

pub(crate) fn choices(i: &Inner, ws: Uuid) -> Result<RepoChoices> {
    i.read(ws, |svc| {
        let roster = svc.workspace_roster(ws)?;
        let maps = svc.store().repo_mappings(ws).unwrap_or_default();
        let views = repo_views(i, svc, ws);
        let admin = svc.my_workspace_role(ws).is_some_and(|r| r.rank() >= ligus_core::WorkspaceRole::Admin.rank());
        let repos = ligus_runtime::repos::live_repos(&roster)
            .into_iter()
            .map(|r| RepoChoice {
                id: r.id.to_string(),
                name: r.display_name.clone(),
                scratch: views.get(&r.id).is_some_and(|v| v.scratch),
                here: maps.iter().any(|m| m.repo_id == r.id),
            })
            .collect();
        let new_scratch = if admin { Allowed::Yes } else { Allowed::No("only an Admin adds repositories, scratch ones too".into()) };
        Ok(RepoChoices { new_scratch, repos })
    })
}

pub(crate) fn set_chat_repo(i: &Inner, ws: Uuid, chat: Uuid, repo: Option<&str>) -> Result<()> {
    let repo = repo.map(|r| Uuid::parse_str(r).map_err(|_| BackendError("no such repository".into()))).transpose()?;
    i.write(ws, |svc| {
        // A live scratch repository of this device follows its chats (the
        // cleanup notices them); any other goes through the chat alone.
        let entry = repo.and_then(|r| scratch::live(svc.store(), ws).into_iter().find(|e| e.repo_id == Some(r)));
        match entry {
            Some(e) => scratch_host::use_in_chat(svc, ws, e.id, chat)
                .map(|_| ())
                .map_err(|e| ligus_runtime::chat_service::ChatError::Unauthorized(e.to_string())),
            None => {
                let base = repo
                    .and_then(|r| svc.workspace_roster(ws).ok()?.repos.get(&r).map(|e| e.repo.default_branch.clone()))
                    .unwrap_or_default();
                svc.set_chat_repo(chat, ws, repo, &base)?;
                for e in scratch::list(svc.store(), ws).into_iter().filter(|e| e.chats.contains(&chat)) {
                    let _ = scratch::set_chat(svc.store(), e.id, chat, false, scratch::now());
                }
                Ok(())
            }
        }
    })
}

/// Make a scratch repository for `chat`, named after the chat.
pub(crate) fn create(i: &Inner, ws: Uuid, chat: Uuid) -> Result<ChatRepo> {
    let name = i.read(ws, |svc| svc.load_in(ws, chat))?.map(|s| s.title).unwrap_or_default();
    let label = i.label();
    let (_, repo) = i.write(ws, |svc| {
        let settings = i.scratch_settings(svc);
        scratch_host::create(svc, ws, Some(chat), &settings, &forbidden(), &name, &label)
            .map_err(|e| ligus_runtime::chat_service::ChatError::Unauthorized(e.to_string()))
    })?;
    i.read(ws, |svc| Ok(repo_views(i, svc, ws).remove(&repo.id)))?.ok_or_else(|| BackendError("the scratch repo vanished".into()))
}

/// Keep `chat`'s scratch repository at `dest` (`~` allowed): moved there,
/// bound there, no longer scratch. Returns where it is now.
pub(crate) fn keep(i: &Inner, ws: Uuid, chat: Uuid, dest: &str) -> Result<String> {
    let dest = crate::setup::complete::expand(dest.trim(), dirs::home_dir().as_deref());
    if dest.as_os_str().is_empty() {
        return err("pick a folder to keep it in");
    }
    let label = i.label();
    let (kept, _) = i.write(ws, |svc| {
        let repo = svc.load_in(ws, chat)?.and_then(|s| s.repo).and_then(|r| r.repo_id);
        let entry = repo.and_then(|r| scratch::live(svc.store(), ws).into_iter().find(|e| e.repo_id == Some(r)));
        let Some(e) = entry else {
            return Err(ligus_runtime::chat_service::ChatError::Unauthorized(
                "this chat's repository isn't a scratch repo this device made".into(),
            ));
        };
        scratch_host::keep(svc, ws, e.id, Path::new(&dest), None, &forbidden(), &label)
            .map_err(|e| ligus_runtime::chat_service::ChatError::Unauthorized(e.to_string()))
    })?;
    Ok(kept.path.display().to_string())
}

/// One cleanup pass for `ws` (a no-op while this device may not sign).
/// Removal only moves a folder to the trash under the service lock; the
/// trash is deleted after, without holding it.
pub(crate) fn sweep(i: &Inner, ws: Uuid) -> Result<scratch_host::SweepReport> {
    let label = i.label();
    let (report, trash) = i
        .write(ws, |svc| {
            let settings = i.scratch_settings(svc);
            let report = scratch_host::sweep(svc, ws, &settings, &label, scratch::now());
            Ok((report, scratch::pending_trash(svc.store())))
        })
        .map_err(|e| BackendError(one_line(e).0))?;
    for t in trash {
        let _ = scratch::delete_trash(&t);
    }
    Ok(report)
}
