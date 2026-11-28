//! What `lt` keeps about itself, device-local, in its data dir.
//!
//! - The workspace list: `workspaces.json` with its secrets in the OS secret
//!   store under scope `term` (`ligus_runtime::connection_settings`).
//! - `lt-state.json` (owner-only): read markers (per workspace, then per
//!   chat), notification history, settings, and which recovery keys were
//!   confirmed written down. The checkout this device applies proposals in,
//!   per workspace, moved to `ligus.db`'s `workspace_folders` table in 0.5
//!   (the one table every Ligus app keeps its workspace folders in);
//!   `checkouts` here is imported once and kept as it was, for a downgrade.
//!
//! Neither file holds a secret.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The secret-store scope of `lt`'s connection secrets (the CLI uses `cli`).
pub const SECRET_SCOPE: &str = "term";

/// The file under the data dir.
pub const STATE_FILE: &str = "lt-state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalState {
    /// Before 0.5: per chat, the newest message time the person has seen
    /// (ms). Kept as it was (a downgrade reads it) and read only where
    /// [`LocalState::read_by_workspace`] has no mark for a chat.
    #[serde(default)]
    pub read: BTreeMap<String, i64>,
    /// Per workspace, then per chat, the newest message time the person has
    /// seen (ms). A chat id is unique only within its workspace.
    #[serde(default)]
    pub read_by_workspace: BTreeMap<String, BTreeMap<String, i64>>,
    #[serde(default)]
    pub notifications: Vec<SavedNotification>,
    #[serde(default = "yes")]
    pub notify_mentions: bool,
    #[serde(default)]
    pub agents_may_edit: bool,
    /// Workspaces whose recovery words the person confirmed writing down.
    #[serde(default)]
    pub recovery_saved: BTreeSet<String>,
    /// Before 0.5: per workspace, the git checkout this device applies
    /// proposals in. Imported once into `workspace_folders`, then only read
    /// while that import hasn't run (`Inner::workspace_folder`).
    #[serde(default)]
    pub checkouts: BTreeMap<String, String>,
    #[serde(default)]
    pub seq: u64,
}

fn yes() -> bool {
    true
}

impl Default for LocalState {
    fn default() -> Self {
        Self {
            read: BTreeMap::new(),
            read_by_workspace: BTreeMap::new(),
            notifications: Vec::new(),
            notify_mentions: true,
            agents_may_edit: false,
            recovery_saved: BTreeSet::new(),
            checkouts: BTreeMap::new(),
            seq: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedNotification {
    pub id: String,
    pub workspace: String,
    #[serde(default)]
    pub chat: Option<String>,
    pub text: String,
    pub at: i64,
    #[serde(default)]
    pub read: bool,
}

/// Most notifications kept.
const MAX_NOTIFICATIONS: usize = 500;

impl LocalState {
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(STATE_FILE)
    }

    pub fn load(data_dir: &Path) -> Self {
        std::fs::read(Self::path(data_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        ligus_runtime::private_file::write_private_atomic(&Self::path(data_dir), &json)
    }

    /// The newest message time seen in `chat` of `workspace` (ms; 0 for
    /// none): this workspace's mark, else the mark from before 0.5.
    pub fn seen(&self, workspace: &str, chat: &str) -> i64 {
        self.read_by_workspace.get(workspace).and_then(|m| m.get(chat)).or_else(|| self.read.get(chat)).copied().unwrap_or(0)
    }

    /// Mark `chat` of `workspace` seen up to `ms`.
    pub fn mark_seen(&mut self, workspace: &str, chat: &str, ms: i64) {
        self.read_by_workspace.entry(workspace.to_string()).or_default().insert(chat.to_string(), ms);
    }

    pub fn push_notification(&mut self, workspace: &str, chat: Option<&str>, text: String, at: i64) -> SavedNotification {
        self.seq += 1;
        let n = SavedNotification {
            id: format!("n{}", self.seq),
            workspace: workspace.to_string(),
            chat: chat.map(str::to_string),
            text,
            at,
            read: false,
        };
        self.notifications.insert(0, n.clone());
        self.notifications.truncate(MAX_NOTIFICATIONS);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_owner_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = LocalState::default();
        s.read.insert("c".into(), 5);
        s.push_notification("w", Some("c"), "hi".into(), 7);
        s.save(tmp.path()).unwrap();
        let back = LocalState::load(tmp.path());
        assert_eq!(back.read.get("c"), Some(&5));
        assert_eq!(back.notifications.len(), 1);
        assert!(back.notify_mentions);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(LocalState::path(tmp.path())).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    /// Read marks are kept per workspace; a mark from before 0.5 (per chat
    /// only) still counts until the chat is read again, and the old field is
    /// never rewritten (a downgrade reads it). An older file still loads.
    #[test]
    fn read_marks_are_per_workspace_and_the_old_ones_still_count() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(LocalState::path(tmp.path()), r#"{"read":{"c1":5,"c2":9},"checkouts":{"w1":"/src/app"}}"#).unwrap();
        let mut s = LocalState::load(tmp.path());
        assert_eq!(s.checkouts.get("w1").map(String::as_str), Some("/src/app"), "an older file loads");
        assert_eq!((s.seen("w1", "c1"), s.seen("w2", "c2"), s.seen("w1", "c3")), (5, 9, 0));
        s.mark_seen("w1", "c1", 50);
        assert_eq!((s.seen("w1", "c1"), s.seen("w2", "c1")), (50, 5), "per workspace");
        s.save(tmp.path()).unwrap();
        let back = LocalState::load(tmp.path());
        assert_eq!(back.seen("w1", "c1"), 50);
        assert_eq!(back.read.get("c1"), Some(&5), "the old marks are kept as they were");
        // A 0.4 lt reads this file: the old shape is still there.
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(LocalState::path(tmp.path())).unwrap()).unwrap();
        assert!(v["read"]["c1"].is_i64() && v["readByWorkspace"].is_null() && v["read_by_workspace"]["w1"]["c1"] == 50);
    }

    #[test]
    fn a_missing_file_is_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let s = LocalState::load(tmp.path());
        assert!(s.notify_mentions && s.notifications.is_empty());
    }
}
