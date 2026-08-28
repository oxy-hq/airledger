//! Local store unit tests — CRUD, tombstones, ordering, pending.

use std::collections::BTreeMap;

use airledger_engine::parse_view;
use airledger_engine::store::Store;
use airledger_engine::value::CellValue;

const WEIGHT_VIEW: &str = r#"
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: date, expr: date }
  - { name: weight_lbs, type: number, expr: weight_lbs }
"#;

fn temp_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join("airledger-store-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}-{}.db", std::process::id()));
    std::fs::remove_file(&path).ok();
    Store::open(path.to_str().unwrap()).unwrap()
}

fn rec(pairs: &[(&str, CellValue)]) -> BTreeMap<String, CellValue> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn create_assigns_id_and_list_round_trips() {
    let store = temp_store("crud");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(180.5))]))
        .unwrap();
    let id = created.get("id").unwrap().to_display_string();
    assert!(!id.is_empty(), "id auto-assigned");
    let listed = store.list(&view, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].get("weight_lbs"), Some(&CellValue::Float(180.5)));
    assert_eq!(store.pending_count().unwrap(), 1, "created row is dirty");
}

#[test]
fn update_overwrites_and_marks_dirty() {
    let store = temp_store("update");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(180.0))]))
        .unwrap();
    let mut edited = created.clone();
    edited.insert("weight_lbs".into(), CellValue::Float(179.0));
    store.update(&view, edited).unwrap();
    let listed = store.list(&view, None).unwrap();
    assert_eq!(listed[0].get("weight_lbs"), Some(&CellValue::Float(179.0)));
}

#[test]
fn delete_of_never_synced_row_removes_outright() {
    let store = temp_store("del-new");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(1.0))]))
        .unwrap();
    store.delete(&view, &created).unwrap();
    assert!(store.list(&view, None).unwrap().is_empty());
    assert_eq!(
        store.pending_count().unwrap(),
        0,
        "no tombstone for unsynced row"
    );
}

#[test]
fn delete_of_synced_row_leaves_tombstone() {
    let store = temp_store("del-synced");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(1.0))]))
        .unwrap();
    let id = created.get("id").unwrap().to_display_string();
    // Simulate a completed sync: base := data, dirty cleared.
    store.mark_synced(&view.name, &id, &created, Some(0)).unwrap();
    assert_eq!(store.pending_count().unwrap(), 0);
    store.delete(&view, &created).unwrap();
    assert!(store.list(&view, None).unwrap().is_empty(), "hidden from list");
    assert_eq!(store.pending_count().unwrap(), 1, "tombstone pending push");
    let rows = store.rows_for_sync("weight").unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].deleted);
}

#[test]
fn list_orders_newest_first() {
    let store = temp_store("order");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(1.0))]))
        .unwrap();
    store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(2.0))]))
        .unwrap();
    let listed = store.list(&view, None).unwrap();
    assert_eq!(listed[0].get("weight_lbs"), Some(&CellValue::Float(2.0)));
}

#[test]
fn open_creates_schema_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("airledger-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t1.db");
    let path_str = path.to_str().unwrap();
    {
        let store = Store::open(path_str).expect("first open");
        assert_eq!(
            store.meta_get("schema_version").unwrap().as_deref(),
            Some("1")
        );
    }
    // Reopen: no error, version unchanged.
    let store = Store::open(path_str).expect("reopen");
    assert_eq!(
        store.meta_get("schema_version").unwrap().as_deref(),
        Some("1")
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn list_filters_by_date_and_sorts_by_log_time() {
    let store = temp_store("datefilter");
    // date_field + plannable only enter a ViewSchema via the input
    // overlay, so build the fixture the way the app does.
    let base = parse_view(
        r#"
name: strength
datasource: gsheets
table: strength
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: date, expr: date }
  - { name: time, type: string, expr: time }
"#,
    )
    .unwrap();
    let overlay = airledger_engine::parse_input_overlay(
        r#"
target: strength.view.yml
date_field: date
plannable:
  log_field: time
  log_format: time_string
"#,
    )
    .unwrap();
    let view = airledger_engine::apply_overlay(base, overlay).unwrap();
    let d = |s: &str| CellValue::Date(chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap());
    store
        .create(&view, rec(&[("date", d("2026-08-27")), ("time", CellValue::String("18:00".into()))]))
        .unwrap();
    store
        .create(&view, rec(&[("date", d("2026-08-27")), ("time", CellValue::String("07:30".into()))]))
        .unwrap();
    store
        .create(&view, rec(&[("date", d("2026-08-26")), ("time", CellValue::String("09:00".into()))]))
        .unwrap();

    let on = chrono::NaiveDate::parse_from_str("2026-08-27", "%Y-%m-%d").unwrap();
    let listed = store.list(&view, Some(on)).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].get("time"), Some(&CellValue::String("07:30".into())));
}
