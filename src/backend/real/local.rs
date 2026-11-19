//! What `lt` keeps about itself, device-local, in its data dir.
//!
//! - The workspace list: `workspaces.json` with its secrets in the OS secret
//!   store under scope `term` (`hive_runtime::connection_settings`).
//! - `lt-state.json` (owner-only): read markers, notification history,
//!   settings, which recovery keys were confirmed written down, and the
//!   checkout this device applies proposals in, per workspace.
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
    /// Per chat, the newest message time the person has seen (ms).
    #[serde(default)]
    pub read: BTreeMap<String, i64>,
    #[serde(default)]
    pub notifications: Vec<SavedNotification>,
    #[serde(default = "yes")]
    pub notify_mentions: bool,
    #[serde(default)]
    pub agents_may_edit: bool,
    /// Workspaces whose recovery words the person confirmed writing down.
    #[serde(default)]
    pub recovery_saved: BTreeSet<String>,
    /// Per workspace, the git checkout this device applies proposals in.
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
        hive_runtime::private_file::write_private_atomic(&Self::path(data_dir), &json)
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

    #[test]
    fn a_missing_file_is_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let s = LocalState::load(tmp.path());
        assert!(s.notify_mentions && s.notifications.is_empty());
    }
}
