//! Sync-engine tests against an in-memory `FakeRemote` — no network.

use std::cell::RefCell;

use airledger_engine::parse_view;
use airledger_engine::sheets::SheetsError;
use airledger_engine::store::Store;
use airledger_engine::sync::{sync_views, SyncRemote};
use airledger_engine::value::{CellValue, Record};
use airledger_engine::ViewSchema;

const WEIGHT_VIEW: &str = r#"
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: weight_lbs, type: number, expr: weight_lbs }
"#;

/// In-memory stand-in for the sheet: rows newest-first, like the
/// real repo's insert-at-row-2.
#[derive(Default)]
struct FakeRemote {
    rows: RefCell<Vec<Record>>,
    fail_pushes: bool,
}

impl FakeRemote {
    fn with_rows(rows: Vec<Record>) -> Self {
        Self { rows: RefCell::new(rows), fail_pushes: false }
    }
}

impl SyncRemote for FakeRemote {
    fn ensure(&self, _view: &ViewSchema) -> Result<(), SheetsError> {
        Ok(())
    }
    fn pull(&self, _view: &ViewSchema) -> Result<Vec<Record>, SheetsError> {
        Ok(self
            .rows
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut r = r.clone();
                r.insert("__row".into(), CellValue::Int(i as i64));
                r
            })
            .collect())
    }
    fn push_update(&self, _view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        if self.fail_pushes {
            return Err(SheetsError::Other("boom".into()));
        }
        let idx = match record.get("__row") {
            Some(CellValue::Int(i)) => *i as usize,
            _ => panic!("push_update without __row"),
        };
        let mut clean = record.clone();
        clean.remove("__row");
        self.rows.borrow_mut()[idx] = clean;
        Ok(())
    }
    fn push_insert(&self, _view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        if self.fail_pushes {
            return Err(SheetsError::Other("boom".into()));
        }
        self.rows.borrow_mut().insert(0, record.clone());
        Ok(())
    }
    fn push_delete(&self, _view: &ViewSchema, row_index: usize) -> Result<(), SheetsError> {
        if self.fail_pushes {
            return Err(SheetsError::Other("boom".into()));
        }
        self.rows.borrow_mut().remove(row_index);
        Ok(())
    }
}

fn temp_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join("airledger-sync-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}-{}.db", std::process::id()));
    std::fs::remove_file(&path).ok();
    Store::open(path.to_str().unwrap()).unwrap()
}

fn rec(id: &str, v: f64) -> Record {
    let mut r = Record::new();
    if !id.is_empty() {
        r.insert("id".into(), CellValue::String(id.into()));
    }
    r.insert("weight_lbs".into(), CellValue::Float(v));
    r
}

#[test]
fn initial_sync_hydrates_empty_store() {
    let store = temp_store("hydrate");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0), rec("B", 2.0)]);
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    assert_eq!(results[0].pulled, 2);
    assert_eq!(store.list(&view, None).unwrap().len(), 2);
    assert_eq!(store.pending_count().unwrap(), 0);
}

#[test]
fn local_create_pushes_and_clears_dirty() {
    let store = temp_store("push");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::default();
    store.create(&view, rec("", 3.0)).unwrap(); // id auto-assigned
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    assert_eq!(results[0].pushed, 1);
    assert_eq!(remote.rows.borrow().len(), 1);
    assert_eq!(store.pending_count().unwrap(), 0);
}

#[test]
fn full_round_trip_bidirectional() {
    let store = temp_store("bidir");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0)]);
    sync_views(&store, &remote, &[view.clone()]);
    // Sheet edit + local create, different rows.
    remote.rows.borrow_mut()[0] = rec("A", 9.0);
    store.create(&view, rec("", 5.0)).unwrap();
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    let local = store.list(&view, None).unwrap();
    assert_eq!(local.len(), 2);
    assert!(
        local
            .iter()
            .any(|r| r.get("weight_lbs") == Some(&CellValue::Float(9.0))),
        "sheet edit pulled"
    );
    assert_eq!(remote.rows.borrow().len(), 2, "local create pushed");
}

#[test]
fn conflict_app_wins() {
    let store = temp_store("conflict");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0)]);
    sync_views(&store, &remote, &[view.clone()]);
    remote.rows.borrow_mut()[0] = rec("A", 100.0); // sheet edit
    let mut edited = store.list(&view, None).unwrap().remove(0);
    edited.insert("weight_lbs".into(), CellValue::Float(50.0));
    store.update(&view, edited).unwrap(); // app edit, same row
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert_eq!(results[0].conflicts, 1);
    assert_eq!(
        remote.rows.borrow()[0].get("weight_lbs"),
        Some(&CellValue::Float(50.0))
    );
}

#[test]
fn tombstone_deletes_remote_and_purges() {
    let store = temp_store("tomb");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0)]);
    sync_views(&store, &remote, &[view.clone()]);
    let row = store.list(&view, None).unwrap().remove(0);
    store.delete(&view, &row).unwrap();
    sync_views(&store, &remote, &[view.clone()]);
    assert!(remote.rows.borrow().is_empty());
    assert_eq!(
        store.rows_for_sync("weight").unwrap().len(),
        0,
        "tombstone purged"
    );
}

#[test]
fn idless_remote_rows_get_ids_written_back() {
    let store = temp_store("idless");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let mut no_id = Record::new();
    no_id.insert("weight_lbs".into(), CellValue::Float(7.0));
    let remote = FakeRemote::with_rows(vec![no_id]);
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    let sheet_id = remote.rows.borrow()[0]
        .get("id")
        .unwrap()
        .to_display_string();
    assert!(!sheet_id.is_empty(), "id written back to sheet");
    let local = store.list(&view, None).unwrap();
    assert_eq!(local[0].get("id").unwrap().to_display_string(), sheet_id);
}

#[test]
fn push_failure_keeps_dirty_for_retry() {
    let store = temp_store("retry");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let mut remote = FakeRemote::default();
    remote.fail_pushes = true;
    store.create(&view, rec("", 1.0)).unwrap();
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_some());
    assert_eq!(store.pending_count().unwrap(), 1, "still dirty");
    // Remote heals → retry succeeds.
    remote.fail_pushes = false;
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none(), "{:?}", results[0].error);
    assert_eq!(store.pending_count().unwrap(), 0);
}
