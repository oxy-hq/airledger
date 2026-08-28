use serde::Serialize;

use crate::schema::view::ViewSchema;
use crate::sheets::{SheetsError, SheetsRepository, ROW_INDEX_KEY};
use crate::store::Store;
use crate::value::{CellValue, Record};

use super::merge::{merge, Action, RemoteRow};

/// The remote side of a sync, abstracted so the engine is testable
/// without network. `pull` returns decoded records carrying `__row`;
/// push ops mirror `SheetsRepository`'s addressing.
pub trait SyncRemote {
    fn ensure(&self, view: &ViewSchema) -> Result<(), SheetsError>;
    fn pull(&self, view: &ViewSchema) -> Result<Vec<Record>, SheetsError>;
    /// Overwrite the row at `record["__row"]` with `record`.
    fn push_update(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError>;
    /// Insert `record` at the top of the sheet (data row 0).
    fn push_insert(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError>;
    fn push_delete(&self, view: &ViewSchema, row_index: usize) -> Result<(), SheetsError>;
}

impl SyncRemote for SheetsRepository {
    fn ensure(&self, view: &ViewSchema) -> Result<(), SheetsError> {
        self.ensure_sheet(view)
    }
    fn pull(&self, view: &ViewSchema) -> Result<Vec<Record>, SheetsError> {
        self.list(view, None)
    }
    fn push_update(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        self.update(view, record.clone())
    }
    fn push_insert(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        let mut r = record.clone();
        r.remove(ROW_INDEX_KEY);
        self.create(view, r).map(|_| ())
    }
    fn push_delete(&self, view: &ViewSchema, row_index: usize) -> Result<(), SheetsError> {
        let mut r = Record::new();
        r.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(row_index as i64));
        self.delete(view, &r)
    }
}

/// Per-view sync outcome — serialized over FFI as the sync summary.
#[derive(Debug, Serialize)]
pub struct ViewSyncResult {
    pub view: String,
    pub pulled: usize,
    pub pushed: usize,
    pub deleted_local: usize,
    pub deleted_remote: usize,
    pub conflicts: usize,
    pub error: Option<String>,
}

/// Sync every view: pull → merge → push → commit, per the spec.
/// Commit is per-action (dirty clears only after that row's push
/// succeeds), so an abort anywhere leaves a retryable state.
/// A view's error aborts that view only; later views still sync.
pub fn sync_views(
    store: &Store,
    remote: &dyn SyncRemote,
    views: &[ViewSchema],
) -> Vec<ViewSyncResult> {
    views.iter().map(|v| sync_one(store, remote, v)).collect()
}

fn sync_one(store: &Store, remote: &dyn SyncRemote, view: &ViewSchema) -> ViewSyncResult {
    let mut res = ViewSyncResult {
        view: view.name.clone(),
        pulled: 0,
        pushed: 0,
        deleted_local: 0,
        deleted_remote: 0,
        conflicts: 0,
        error: None,
    };
    if let Err(e) = sync_one_inner(store, remote, view, &mut res) {
        res.error = Some(e);
    }
    res
}

fn sync_one_inner(
    store: &Store,
    remote: &dyn SyncRemote,
    view: &ViewSchema,
    res: &mut ViewSyncResult,
) -> Result<(), String> {
    remote.ensure(view).map_err(|e| format!("ensure: {e}"))?;

    // Pull + normalize: strip __row into row_index, assign ids to
    // id-less rows (written back immediately so the sheet row is
    // addressable), dedup ids (first wins).
    let pulled = remote.pull(view).map_err(|e| format!("pull: {e}"))?;
    let mut remote_rows: Vec<RemoteRow> = Vec::new();
    let mut seen_ids = std::collections::BTreeSet::new();
    for mut rec in pulled {
        let row_index = match rec.remove(ROW_INDEX_KEY) {
            Some(CellValue::Int(i)) => i as usize,
            _ => continue,
        };
        let id = rec
            .get("id")
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        let id = if id.is_empty() {
            let new_id = uuid::Uuid::new_v4().to_string();
            rec.insert("id".into(), CellValue::String(new_id.clone()));
            let mut with_row = rec.clone();
            with_row.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(row_index as i64));
            remote
                .push_update(view, &with_row)
                .map_err(|e| format!("id write-back: {e}"))?;
            new_id
        } else {
            id
        };
        if !seen_ids.insert(id.clone()) {
            continue; // duplicate id in sheet — first wins
        }
        remote_rows.push(RemoteRow { id, data: rec, row_index });
    }

    let local = store
        .rows_for_sync(&view.name)
        .map_err(|e| format!("local read: {e}"))?;
    let local_by_id: std::collections::BTreeMap<&str, &crate::store::LocalRow> =
        local.iter().map(|l| (l.id.as_str(), l)).collect();
    let plan = merge(&local, &remote_rows);
    res.conflicts = plan.conflicts;

    // Phase A — remote pushes only, no store writes. Ordering
    // contract (see merge::Action docs): updates while the snapshot
    // indexes are valid, then deletes bottom-up, then inserts. Each
    // successful push queues its local commit; the first failure
    // stops pushing but the successes still commit below.
    let mut updates = Vec::new();
    let mut deletes = Vec::new();
    let mut inserts = Vec::new();
    let mut commits: Vec<CommitOp> = Vec::new();
    for a in &plan.actions {
        match a {
            Action::TakeRemote { id, data, row_index } => {
                commits.push(CommitOp::MarkSynced {
                    id: id.clone(),
                    data: data.clone(),
                    sort_key: Some(*row_index as i64),
                    pulled: true,
                });
            }
            Action::DeleteLocal { id } => {
                commits.push(CommitOp::Remove { id: id.clone(), remote: false });
            }
            Action::PushUpdate { .. } => updates.push(a.clone()),
            Action::DeleteRemote { .. } => deletes.push(a.clone()),
            Action::PushInsert { .. } => inserts.push(a.clone()),
        }
    }

    deletes.sort_by_key(|a| {
        let Action::DeleteRemote { row_index, .. } = a else {
            unreachable!()
        };
        std::cmp::Reverse(*row_index)
    });

    let mut push_err: Option<String> = None;
    'push: {
        for a in updates {
            let Action::PushUpdate { id, row_index } = a else {
                unreachable!()
            };
            let l = local_by_id
                .get(id.as_str())
                .expect("merge only names local ids");
            let mut rec = l.data.clone();
            rec.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(row_index as i64));
            if let Err(e) = remote.push_update(view, &rec) {
                push_err = Some(format!("push update: {e}"));
                break 'push;
            }
            commits.push(CommitOp::MarkSynced {
                id,
                data: l.data.clone(),
                sort_key: Some(row_index as i64),
                pulled: false,
            });
        }
        for a in deletes {
            let Action::DeleteRemote { id, row_index } = a else {
                unreachable!()
            };
            if let Err(e) = remote.push_delete(view, row_index) {
                push_err = Some(format!("push delete: {e}"));
                break 'push;
            }
            commits.push(CommitOp::Remove { id, remote: true });
        }
        for a in inserts {
            let Action::PushInsert { id } = a else {
                unreachable!()
            };
            let l = local_by_id
                .get(id.as_str())
                .expect("merge only names local ids");
            if let Err(e) = remote.push_insert(view, &l.data) {
                push_err = Some(format!("push insert: {e}"));
                break 'push;
            }
            commits.push(CommitOp::MarkSynced {
                id,
                data: l.data.clone(),
                sort_key: None,
                pulled: false,
            });
        }
    }

    // Phase B — one write transaction for every local mutation this
    // sync earned: pulled rows, push commits, sheet-order keys. A
    // 33k-row hydration lands as a single fsync instead of one per
    // row, and the UI's connection is blocked for milliseconds, not
    // the sync's whole lifetime.
    store
        .tx(|s| {
            for op in &commits {
                match op {
                    CommitOp::MarkSynced { id, data, sort_key, pulled } => {
                        s.mark_synced(&view.name, id, data, *sort_key)?;
                        if *pulled {
                            res.pulled += 1;
                        } else {
                            res.pushed += 1;
                        }
                    }
                    CommitOp::Remove { id, remote: was_remote } => {
                        s.remove(&view.name, id)?;
                        if *was_remote {
                            res.deleted_remote += 1;
                        } else {
                            res.deleted_local += 1;
                        }
                    }
                }
            }
            for r in &remote_rows {
                s.set_sort_key(&view.name, &r.id, r.row_index as i64)?;
            }
            if push_err.is_none() {
                s.meta_set(
                    &format!("last_sync_{}", view.name),
                    &chrono::Utc::now().to_rfc3339(),
                )?;
            }
            Ok(())
        })
        .map_err(|e| format!("commit: {e}"))?;

    match push_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// A deferred local mutation, applied in the phase-B transaction.
enum CommitOp {
    MarkSynced {
        id: String,
        data: Record,
        sort_key: Option<i64>,
        pulled: bool,
    },
    Remove {
        id: String,
        remote: bool,
    },
}
