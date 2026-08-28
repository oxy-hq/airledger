//! Table-driven tests for the pure three-way merge — every row of
//! the spec matrix (docs/superpowers/specs/2026-08-27-local-first-
//! sync-design.md §2) plus the edge rows.

use airledger_engine::store::LocalRow;
use airledger_engine::sync::{merge, Action, RemoteRow};
use airledger_engine::value::{CellValue, Record};

fn r(v: f64) -> Record {
    [
        ("id".to_string(), CellValue::String("A".into())),
        ("weight_lbs".to_string(), CellValue::Float(v)),
    ]
    .into_iter()
    .collect()
}

fn local(data: Record, base: Option<Record>, dirty: bool, deleted: bool) -> LocalRow {
    LocalRow { id: "A".into(), data, base, dirty, deleted }
}

fn remote(data: Record) -> RemoteRow {
    RemoteRow { id: "A".into(), data, row_index: 0 }
}

#[test]
fn clean_remote_changed_takes_remote() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, false)], &[remote(r(2.0))]);
    assert!(matches!(plan.actions[..], [Action::TakeRemote { .. }]));
    assert_eq!(plan.conflicts, 0);
}

#[test]
fn dirty_remote_unchanged_pushes_local() {
    let plan = merge(&[local(r(2.0), Some(r(1.0)), true, false)], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::PushUpdate { .. }]));
    assert_eq!(plan.conflicts, 0);
}

#[test]
fn dirty_remote_changed_app_wins_and_counts_conflict() {
    let plan = merge(&[local(r(3.0), Some(r(1.0)), true, false)], &[remote(r(2.0))]);
    assert!(matches!(plan.actions[..], [Action::PushUpdate { .. }]));
    assert_eq!(plan.conflicts, 1);
}

#[test]
fn clean_remote_gone_deletes_locally() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, false)], &[]);
    assert!(matches!(plan.actions[..], [Action::DeleteLocal { .. }]));
}

#[test]
fn tombstone_deletes_remote_row() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, true)], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::DeleteRemote { .. }]));
}

#[test]
fn tombstone_with_remote_already_gone_purges_locally() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, true)], &[]);
    assert!(matches!(plan.actions[..], [Action::DeleteLocal { .. }]));
}

#[test]
fn new_local_row_inserts_remotely() {
    let plan = merge(&[local(r(1.0), None, true, false)], &[]);
    assert!(matches!(plan.actions[..], [Action::PushInsert { .. }]));
}

#[test]
fn remote_only_row_is_pulled() {
    let plan = merge(&[], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::TakeRemote { .. }]));
}

#[test]
fn clean_unchanged_yields_no_action() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, false)], &[remote(r(1.0))]);
    assert!(plan.actions.is_empty());
}

#[test]
fn dirty_with_remote_gone_reinserts_app_wins() {
    let plan = merge(&[local(r(2.0), Some(r(1.0)), true, false)], &[]);
    assert!(matches!(plan.actions[..], [Action::PushInsert { .. }]));
    assert_eq!(plan.conflicts, 1);
}

#[test]
fn new_local_row_already_present_remotely_identical_commits_without_insert() {
    // Crash window: push_insert succeeded but the local commit didn't
    // land. Next sync must NOT insert a duplicate — identical remote
    // row with the same id just commits locally.
    let plan = merge(&[local(r(1.0), None, true, false)], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::TakeRemote { .. }]));
    assert_eq!(plan.conflicts, 0);
}

#[test]
fn new_local_row_id_collision_different_data_app_wins() {
    let plan = merge(&[local(r(2.0), None, true, false)], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::PushUpdate { .. }]));
    assert_eq!(plan.conflicts, 1);
}
