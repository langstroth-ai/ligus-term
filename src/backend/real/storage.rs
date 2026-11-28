//! lt's share of the local database steps, run once at open while lt has
//! `ligus.db` alone (another lt or `ligus` process holding it: a later
//! start runs them; until then readers use the older sources).
//!
//! - The sync state keyed by relay and workspace (identity model v2, M1).
//! - Tombstones for pruned events and the messages dispatch handled
//!   (`user_version` 2, workspace model v3 M8 and M10).
//! - The scratch repository list and the workspace folders as tables (M9):
//!   lt-state.json's `checkouts` go into `workspace_folders` once and are
//!   then kept as they were, for a downgrade.
//!
//! Each step takes its own backup first and changes nothing when it fails.

use std::collections::BTreeMap;
use std::path::Path;

use ligus_runtime::EventStore;
use ligus_runtime::connection_settings::WorkspaceList;
use ligus_runtime::event_store::{LegacyFolder, LegacyLocal, LocalTablesMigration};
use uuid::Uuid;

/// The name lt's workspace-folder rows and import carry.
pub(crate) const FOLDER_SOURCE: &str = "lt";

/// Run the steps on the database at `db`. What M9 did, for tests; `None`
/// when the database couldn't be opened or is a newer one.
pub(crate) fn migrate(db: &Path, list: &WorkspaceList, checkouts: &BTreeMap<String, String>) -> Option<LocalTablesMigration> {
    if !db.exists() {
        return None;
    }
    let store = EventStore::open(db).ok()?;
    if store.newer_schema().is_some() {
        return None;
    }
    let conns: Vec<(String, String)> = list.workspaces.iter().map(|w| (w.relay_url.clone(), w.room.clone())).collect();
    let _ = store.migrate_sync_tables(&conns);
    let _ = store.migrate_log_hygiene();
    let folders = checkouts
        .iter()
        .filter_map(|(ws, path)| Some(LegacyFolder { workspace_id: Uuid::parse_str(ws).ok()?, path: path.clone() }))
        .collect();
    store.migrate_local_tables(&LegacyLocal { source: FOLDER_SOURCE, folders }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// lt's checkouts go into the table once; the table is what lt reads
    /// from then on, and lt-state.json is never rewritten for it.
    #[test]
    fn checkouts_move_into_the_table_once() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("ligus.db");
        drop(EventStore::open(&db).unwrap());
        let ws = Uuid::new_v4();
        let checkouts = BTreeMap::from([(ws.to_string(), "/src/app".to_string()), ("junk".into(), "/x".into())]);
        let out = migrate(&db, &WorkspaceList::default(), &checkouts).unwrap();
        assert!(matches!(out, LocalTablesMigration::Migrated { folders: 1, .. }), "{out:?}");
        assert_eq!(ligus_runtime::event_store::peek_workspace_folder(&db, ws, FOLDER_SOURCE), Some(Some(PathBuf::from("/src/app"))));
        assert_eq!(migrate(&db, &WorkspaceList::default(), &checkouts), Some(LocalTablesMigration::AlreadyDone));
    }
}
