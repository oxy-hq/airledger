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

#[test]
fn partial_push_failure_commits_successes_keeps_rest_dirty() {
    let store = temp_store("partial");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FailAfterOne::default();
    store.create(&view, rec("", 1.0)).unwrap();
    store.create(&view, rec("", 2.0)).unwrap();
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_some(), "second push should fail");
    assert_eq!(
        store.pending_count().unwrap(),
        1,
        "the push that succeeded must be committed; the failed one stays dirty"
    );
    assert_eq!(remote.rows.borrow().len(), 1);
}

// ------------------------------------------------------------------
// Codec-faithful remote: reproduces the 2026-09-21 sync storm.
// ------------------------------------------------------------------

const MEALS_VIEW: &str = r#"
name: meals
datasource: gsheets
table: meals
dimensions:
  - { name: id, type: string, expr: id }
  - { name: eaten_at, type: datetime, expr: eaten_at }
  - { name: meal, type: string, expr: meal }
  - { name: protein_g, type: number, expr: protein_g }
  - { name: hc_id, type: string, expr: hc_id }
"#;

/// Remote that stores cells the way Google Sheets does after a
/// USER_ENTERED write, and hands them back the way FORMATTED_VALUE
/// reads do: every value is a string; datetime-looking strings were
/// parsed into native datetime cells and re-render SPACE-separated.
#[derive(Default)]
struct SheetsFaithfulRemote {
    rows: RefCell<Vec<std::collections::BTreeMap<String, String>>>,
}

impl SheetsFaithfulRemote {
    /// USER_ENTERED + FORMATTED_VALUE round-trip for one wire value.
    fn cell_render(v: &CellValue) -> String {
        let s = v.to_display_string();
        // Sheets parses ISO-T datetimes into native datetime cells and
        // renders them back with a space.
        if chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%dT%H:%M:%S").is_ok() {
            return s.replace('T', " ");
        }
        s
    }
    fn store_record(view: &ViewSchema, record: &Record) -> std::collections::BTreeMap<String, String> {
        view.dimensions
            .iter()
            .map(|d| {
                let raw = record.get(&d.name).cloned().unwrap_or(CellValue::Null);
                let enc = airledger_engine::encode(d.kind, &raw);
                (d.name.clone(), Self::cell_render(&enc))
            })
            .collect()
    }
}

impl SyncRemote for SheetsFaithfulRemote {
    fn ensure(&self, _view: &ViewSchema) -> Result<(), SheetsError> {
        Ok(())
    }
    fn pull(&self, view: &ViewSchema) -> Result<Vec<Record>, SheetsError> {
        Ok(self
            .rows
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, cells)| {
                let mut r = Record::new();
                for d in &view.dimensions {
                    let raw = cells.get(&d.name).cloned().unwrap_or_default();
                    r.insert(d.name.clone(), airledger_engine::decode(d.kind, &raw));
                }
                r.insert("__row".into(), CellValue::Int(i as i64));
                r
            })
            .collect())
    }
    fn push_update(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        let idx = match record.get("__row") {
            Some(CellValue::Int(i)) => *i as usize,
            _ => panic!("push_update without __row"),
        };
        self.rows.borrow_mut()[idx] = Self::store_record(view, record);
        Ok(())
    }
    fn push_insert(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        self.rows.borrow_mut().insert(0, Self::store_record(view, record));
        Ok(())
    }
    fn push_delete(&self, _view: &ViewSchema, row_index: usize) -> Result<(), SheetsError> {
        self.rows.borrow_mut().remove(row_index);
        Ok(())
    }
}

#[test]
fn ingested_datetime_row_reaches_steady_state_through_real_codec() {
    // End-to-end reproduction of the 429 push storm: a source ingest
    // creates a row carrying a datetime + an integral macro, sync
    // pushes it, and from then on NOTHING must move — the pull must
    // decode the row back to exactly what was pushed (no TakeRemote),
    // and replaying the identical source batch must not re-dirty it.
    use airledger_engine::store::{ingest, IngestBatch};

    let store = temp_store("steady");
    let view = parse_view(MEALS_VIEW).unwrap();
    let remote = SheetsFaithfulRemote::default();

    let batch: IngestBatch = serde_json::from_str(
        r#"{
        "source": "macrofactor",
        "match_field": "hc_id",
        "owned_fields": ["hc_id", "eaten_at", "protein_g"],
        "fill_if_blank_fields": ["meal"],
        "records": [{
            "hc_id": {"kind":"string","value":"hc-1"},
            "eaten_at": {"kind":"date_time","value":"2026-09-16T10:00:00"},
            "meal": {"kind":"string","value":"Morning Smoothie"},
            "protein_g": {"kind":"float","value":26.0}
        }]
    }"#,
    )
    .unwrap();

    // A date_field is required by ingest; give the view one.
    let mut view = view;
    view.date_field = Some("eaten_at".into());

    ingest(&store, &view, &batch).unwrap();
    assert_eq!(store.pending_count().unwrap(), 1);

    // Sync 1: pushes the new row.
    let res = sync_views(&store, &remote, &[view.clone()]);
    assert!(res[0].error.is_none(), "{:?}", res[0].error);
    assert_eq!(res[0].pushed, 1);
    assert_eq!(store.pending_count().unwrap(), 0);

    // Sync 2: steady state — nothing pushed, nothing pulled-over.
    let res = sync_views(&store, &remote, &[view.clone()]);
    assert!(res[0].error.is_none(), "{:?}", res[0].error);
    assert_eq!(
        (res[0].pushed, res[0].pulled),
        (0, 0),
        "sheet round-trip must be stable — this loop was the 429 storm"
    );

    // The local row still carries a real datetime (not Null).
    let row = &store.list(&view, None).unwrap()[0];
    assert!(
        matches!(row.get("eaten_at"), Some(CellValue::DateTime(_))),
        "eaten_at must survive the pull, got {:?}",
        row.get("eaten_at")
    );

    // Replaying the same source batch must be a no-op.
    let res = ingest(&store, &view, &batch).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 0, 1));
    assert_eq!(store.pending_count().unwrap(), 0, "replay must not re-dirty");
}

/// Remote that lets exactly one push through, then errors.
#[derive(Default)]
struct FailAfterOne {
    rows: RefCell<Vec<Record>>,
    pushes: std::cell::Cell<usize>,
}

impl FailAfterOne {
    fn gate(&self) -> Result<(), SheetsError> {
        let n = self.pushes.get();
        self.pushes.set(n + 1);
        if n >= 1 {
            return Err(SheetsError::Other("boom".into()));
        }
        Ok(())
    }
}

impl SyncRemote for FailAfterOne {
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
        self.gate()?;
        let idx = match record.get("__row") {
            Some(CellValue::Int(i)) => *i as usize,
            _ => panic!(),
        };
        let mut clean = record.clone();
        clean.remove("__row");
        self.rows.borrow_mut()[idx] = clean;
        Ok(())
    }
    fn push_insert(&self, _view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        self.gate()?;
        self.rows.borrow_mut().insert(0, record.clone());
        Ok(())
    }
    fn push_delete(&self, _view: &ViewSchema, row_index: usize) -> Result<(), SheetsError> {
        self.gate()?;
        self.rows.borrow_mut().remove(row_index);
        Ok(())
    }
}
