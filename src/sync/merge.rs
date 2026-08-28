use std::collections::BTreeMap;

use crate::store::LocalRow;
use crate::value::Record;

/// One decoded remote row with its zero-based data row index.
#[derive(Debug, Clone)]
pub struct RemoteRow {
    pub id: String,
    pub data: Record,
    pub row_index: usize,
}

/// One reconciliation decision. Remote-touching actions carry the
/// row index from the pull snapshot; the executor must apply
/// `PushUpdate`s before `DeleteRemote`s (descending) before
/// `PushInsert`s so snapshot indexes stay valid.
#[derive(Debug, Clone)]
pub enum Action {
    /// Remote changed, local clean — overwrite local with remote.
    TakeRemote { id: String, data: Record, row_index: usize },
    /// Local dirty — overwrite the sheet row with local data.
    PushUpdate { id: String, row_index: usize },
    /// Locally new (or app-wins resurrection) — insert at sheet top.
    PushInsert { id: String },
    /// Row gone remotely (or tombstone with remote gone) — drop local.
    DeleteLocal { id: String },
    /// Local tombstone — delete the sheet row, then drop local.
    DeleteRemote { id: String, row_index: usize },
}

#[derive(Debug, Default)]
pub struct MergePlan {
    pub actions: Vec<Action>,
    /// Rows where both sides changed since last sync (app won).
    pub conflicts: usize,
}

/// Pure three-way merge: local rows (with `base` = last-synced remote
/// copy) vs the current remote snapshot. No I/O. Spec matrix:
/// docs/superpowers/specs/2026-08-27-local-first-sync-design.md §2.
pub fn merge(local: &[LocalRow], remote: &[RemoteRow]) -> MergePlan {
    let remote_by_id: BTreeMap<&str, &RemoteRow> =
        remote.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut plan = MergePlan::default();
    let mut seen = std::collections::BTreeSet::new();

    for l in local {
        seen.insert(l.id.as_str());
        let r = remote_by_id.get(l.id.as_str()).copied();
        if l.deleted {
            match r {
                Some(r) => plan.actions.push(Action::DeleteRemote {
                    id: l.id.clone(),
                    row_index: r.row_index,
                }),
                None => plan.actions.push(Action::DeleteLocal { id: l.id.clone() }),
            }
            continue;
        }
        match (&l.base, r) {
            // Never synced with no remote counterpart: push as a new
            // sheet row.
            (None, None) => plan.actions.push(Action::PushInsert { id: l.id.clone() }),
            // Never synced but the id already exists remotely — the
            // crash window between a successful push_insert and its
            // local commit. Identical data: just commit locally.
            // Divergent data: app wins, overwrite in place. Either
            // way, never insert a duplicate row.
            (None, Some(r)) => {
                if r.data == l.data {
                    plan.actions.push(Action::TakeRemote {
                        id: l.id.clone(),
                        data: r.data.clone(),
                        row_index: r.row_index,
                    });
                } else {
                    plan.conflicts += 1;
                    plan.actions.push(Action::PushUpdate {
                        id: l.id.clone(),
                        row_index: r.row_index,
                    });
                }
            }
            (Some(base), Some(r)) => {
                let remote_changed = r.data != *base;
                match (l.dirty, remote_changed) {
                    (false, false) => {}
                    (false, true) => plan.actions.push(Action::TakeRemote {
                        id: l.id.clone(),
                        data: r.data.clone(),
                        row_index: r.row_index,
                    }),
                    (true, false) => plan.actions.push(Action::PushUpdate {
                        id: l.id.clone(),
                        row_index: r.row_index,
                    }),
                    (true, true) => {
                        plan.conflicts += 1;
                        plan.actions.push(Action::PushUpdate {
                            id: l.id.clone(),
                            row_index: r.row_index,
                        });
                    }
                }
            }
            (Some(_), None) => {
                if l.dirty {
                    // Deleted in the sheet while edited in the app —
                    // app wins: resurrect as a new row.
                    plan.conflicts += 1;
                    plan.actions.push(Action::PushInsert { id: l.id.clone() });
                } else {
                    plan.actions.push(Action::DeleteLocal { id: l.id.clone() });
                }
            }
        }
    }

    for r in remote {
        if !seen.contains(r.id.as_str()) {
            plan.actions.push(Action::TakeRemote {
                id: r.id.clone(),
                data: r.data.clone(),
                row_index: r.row_index,
            });
        }
    }
    plan
}
