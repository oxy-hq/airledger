//! Ingest primitive tests — merge rules, idempotency, deletion unwind.

use airledger_engine::store::{ingest, IngestBatch, Store};
use airledger_engine::value::CellValue;
use airledger_engine::{apply_overlay, parse_input_overlay, parse_view};

fn weight_view() -> airledger_engine::ViewSchema {
    let base = parse_view(
        "name: weight\ndatasource: gsheets\ntable: weight\ndimensions:\n  - { name: id, type: string, expr: id }\n  - { name: date, type: date, expr: date }\n  - { name: time, type: string, expr: time }\n  - { name: weight_lbs, type: number, expr: weight_lbs }\n  - { name: body_fat_withing, type: number, expr: body_fat_withing }\n",
    )
    .unwrap();
    let overlay =
        parse_input_overlay("target: weight.view.yml\ndate_field: date\n").unwrap();
    apply_overlay(base, overlay).unwrap()
}

fn temp_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join("airledger-ingest-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}-{}.db", std::process::id()));
    std::fs::remove_file(&path).ok();
    Store::open(path.to_str().unwrap()).unwrap()
}

fn batch(json: &str) -> IngestBatch {
    serde_json::from_str(json).unwrap()
}

const DAY_BATCH: &str = r#"{
  "source": "withings",
  "owned_fields": ["body_fat_withing"],
  "fill_if_blank_fields": ["weight_lbs", "time"],
  "records": [{
    "date": {"kind":"date","value":"2026-08-28"},
    "time": {"kind":"string","value":"07:31"},
    "weight_lbs": {"kind":"float","value":180.9},
    "body_fat_withing": {"kind":"float","value":18.2}
  }]
}"#;

#[test]
fn creates_row_when_day_missing() {
    let store = temp_store("create");
    let view = weight_view();
    let res = ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (1, 0, 0));
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("body_fat_withing"), Some(&CellValue::Float(18.2)));
    assert!(!rows[0].get("id").unwrap().to_display_string().is_empty());
    assert_eq!(store.pending_count().unwrap(), 1, "created row is dirty → syncs");
}

#[test]
fn merges_into_existing_day_row_without_clobbering_manual_values() {
    let store = temp_store("merge");
    let view = weight_view();
    // Manual row: user weighed 180.5, no body fat.
    let mut manual = std::collections::BTreeMap::new();
    manual.insert(
        "date".to_string(),
        CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()),
    );
    manual.insert("weight_lbs".to_string(), CellValue::Float(180.5));
    store.create(&view, manual).unwrap();

    let res = ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 1, 0));
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(180.5)), "manual wins");
    assert_eq!(row.get("body_fat_withing"), Some(&CellValue::Float(18.2)), "owned written");
    assert_eq!(row.get("time"), Some(&CellValue::String("07:31".into())), "blank filled");
}

#[test]
fn replay_is_a_no_op_and_does_not_dirty() {
    let store = temp_store("replay");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    // Pretend sync ran: clear dirty.
    let row = store.list(&view, None).unwrap().remove(0);
    let id = row.get("id").unwrap().to_display_string();
    store.mark_synced(&view.name, &id, &row, Some(0)).unwrap();
    assert_eq!(store.pending_count().unwrap(), 0);

    let res = ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 0, 1));
    assert_eq!(store.pending_count().unwrap(), 0, "no-op must not re-dirty");
}

#[test]
fn owned_field_updates_when_source_value_changes() {
    let store = temp_store("owned");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    let changed = DAY_BATCH.replace("18.2", "17.9");
    let res = ingest(&store, &view, &batch(&changed)).unwrap();
    assert_eq!(res.updated, 1);
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("body_fat_withing"), Some(&CellValue::Float(17.9)));
}

#[test]
fn record_without_date_is_skipped() {
    let store = temp_store("nodate");
    let view = weight_view();
    let b = batch(
        r#"{"source":"withings","records":[{"weight_lbs":{"kind":"float","value":1.0}}]}"#,
    );
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.skipped, 1);
    assert!(store.list(&view, None).unwrap().is_empty());
}
