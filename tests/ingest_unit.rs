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

#[test]
fn deleted_date_removes_source_created_untouched_row() {
    let store = temp_store("del-created");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.deleted, 1);
    assert!(store.list(&view, None).unwrap().is_empty());
}

#[test]
fn deleted_date_clears_only_source_fields_on_manual_row() {
    let store = temp_store("del-manual");
    let view = weight_view();
    let mut manual = std::collections::BTreeMap::new();
    manual.insert(
        "date".to_string(),
        CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()),
    );
    manual.insert("weight_lbs".to_string(), CellValue::Float(180.5));
    store.create(&view, manual).unwrap();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap(); // fills body_fat + time

    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.cleared, 1);
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(180.5)), "manual survives");
    assert!(row.get("body_fat_withing").map_or(true, |v| v.is_empty()), "owned cleared");
    assert!(row.get("time").map_or(true, |v| v.is_empty()), "filled field cleared");
}

#[test]
fn deleted_date_leaves_row_edited_after_ingest_but_clears_fields() {
    let store = temp_store("del-edited");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    // User edits the source-created row afterwards.
    let mut row = store.list(&view, None).unwrap().remove(0);
    row.insert("weight_lbs".into(), CellValue::Float(181.0));
    store.update(&view, row).unwrap();

    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 1), "edited row must not be deleted");
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(181.0)));
}

// Ghost weigh-in scenario: the source created the day's row with a bad
// weight (fill-if-blank field), then later reports the corrected value.
const GHOST_BATCH: &str = r#"{
  "source": "withings",
  "owned_fields": ["body_fat_withing"],
  "fill_if_blank_fields": ["weight_lbs", "time"],
  "records": [{
    "date": {"kind":"date","value":"2026-09-01"},
    "time": {"kind":"string","value":"06:02"},
    "weight_lbs": {"kind":"float","value":20.9}
  }]
}"#;

#[test]
fn fill_if_blank_revises_sources_own_unedited_value() {
    let store = temp_store("revise");
    let view = weight_view();
    ingest(&store, &view, &batch(GHOST_BATCH)).unwrap();
    let corrected = GHOST_BATCH.replace("20.9", "163.4");
    let res = ingest(&store, &view, &batch(&corrected)).unwrap();
    assert_eq!((res.updated, res.unchanged), (1, 0), "revision must land as an update");
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(163.4)));
}

#[test]
fn fill_if_blank_never_clobbers_user_edit() {
    let store = temp_store("useredit");
    let view = weight_view();
    ingest(&store, &view, &batch(GHOST_BATCH)).unwrap();
    // User corrects the weight by hand.
    let mut row = store.list(&view, None).unwrap().remove(0);
    row.insert("weight_lbs".into(), CellValue::Float(165.0));
    store.update(&view, row).unwrap();

    let corrected = GHOST_BATCH.replace("20.9", "163.4");
    ingest(&store, &view, &batch(&corrected)).unwrap();
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(165.0)), "user edit wins");
}

#[test]
fn fill_if_blank_still_fills_blank() {
    let store = temp_store("fillblank");
    let view = weight_view();
    // Row created by another path, weight left blank.
    let mut manual = std::collections::BTreeMap::new();
    manual.insert(
        "date".to_string(),
        CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()),
    );
    store.create(&view, manual).unwrap();

    let res = ingest(&store, &view, &batch(GHOST_BATCH)).unwrap();
    assert_eq!(res.updated, 1);
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(20.9)), "blank filled");
}

#[test]
fn provenance_merges_on_update() {
    let store = temp_store("prov-merge");
    let view = weight_view();
    // Source creates the row with date + weight only.
    let create = r#"{
      "source": "withings",
      "owned_fields": ["body_fat_withing"],
      "fill_if_blank_fields": ["weight_lbs", "time"],
      "records": [{
        "date": {"kind":"date","value":"2026-09-01"},
        "weight_lbs": {"kind":"float","value":163.4}
      }]
    }"#;
    ingest(&store, &view, &batch(create)).unwrap();
    // Later pull writes only a second owned field.
    let update = r#"{
      "source": "withings",
      "owned_fields": ["body_fat_withing"],
      "fill_if_blank_fields": ["weight_lbs", "time"],
      "records": [{
        "date": {"kind":"date","value":"2026-09-01"},
        "body_fat_withing": {"kind":"float","value":18.2}
      }]
    }"#;
    let res = ingest(&store, &view, &batch(update)).unwrap();
    assert_eq!(res.updated, 1);

    // The source deletes the day: the whole row must unwind, which
    // requires prov.created and the earlier-written fields to survive
    // the second provenance write.
    let b = batch(r#"{"source":"withings","deleted_dates":["2026-09-01"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (1, 0), "untouched source row deletes whole");
    assert!(store.list(&view, None).unwrap().is_empty());
}

#[test]
fn deleted_date_without_provenance_is_ignored() {
    let store = temp_store("del-none");
    let view = weight_view();
    let mut manual = std::collections::BTreeMap::new();
    manual.insert(
        "date".to_string(),
        CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()),
    );
    store.create(&view, manual).unwrap();
    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28","2026-08-01"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 0));
    assert_eq!(store.list(&view, None).unwrap().len(), 1);
}

fn climbing_view() -> airledger_engine::ViewSchema {
    let base = parse_view(
        "name: climbing\ndatasource: gsheets\ntable: climbing\ndimensions:\n  - { name: id, type: string, expr: id }\n  - { name: kaya_id, type: string, expr: kaya_id }\n  - { name: date, type: date, expr: date }\n  - { name: climb_name, type: string, expr: climb_name }\n  - { name: grade, type: string, expr: grade }\n  - { name: notes, type: string, expr: notes }\n",
    )
    .unwrap();
    let overlay =
        parse_input_overlay("target: climbing.view.yml\ndate_field: date\n").unwrap();
    apply_overlay(base, overlay).unwrap()
}

const ASCENTS_BATCH: &str = r#"{
  "source": "kaya",
  "match_field": "kaya_id",
  "owned_fields": ["kaya_id", "date", "climb_name", "grade"],
  "fill_if_blank_fields": ["notes"],
  "records": [
    {"kaya_id":{"kind":"string","value":"a1"},"date":{"kind":"date","value":"2026-09-14"},"climb_name":{"kind":"string","value":"Moonwalk"},"grade":{"kind":"string","value":"V5"}},
    {"kaya_id":{"kind":"string","value":"a2"},"date":{"kind":"date","value":"2026-09-14"},"climb_name":{"kind":"string","value":"Slab City"},"grade":{"kind":"string","value":"V3"}}
  ]
}"#;

#[test]
fn match_field_creates_multiple_rows_on_one_day() {
    let store = temp_store("mf-create");
    let view = climbing_view();
    let res = ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.skipped), (2, 0, 0));
    assert_eq!(store.list(&view, None).unwrap().len(), 2, "same day, two rows");
}

#[test]
fn match_field_replay_is_noop() {
    let store = temp_store("mf-replay");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    let res = ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 0, 2));
}

#[test]
fn match_field_upserts_by_id_even_when_date_changes() {
    let store = temp_store("mf-upsert");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    // Kaya revises a2: new grade AND moved to another day. Build the
    // revised batch by editing a2's record fields.
    let revised = ASCENTS_BATCH
        .replace("\"V3\"", "\"V4\"")
        .replace(
            "{\"kaya_id\":{\"kind\":\"string\",\"value\":\"a2\"},\"date\":{\"kind\":\"date\",\"value\":\"2026-09-14\"}",
            "{\"kaya_id\":{\"kind\":\"string\",\"value\":\"a2\"},\"date\":{\"kind\":\"date\",\"value\":\"2026-09-15\"}",
        );
    let res = ingest(&store, &view, &batch(&revised)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 1, 1));
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 2, "revision matched by id, no duplicate row");
    let a2 = rows
        .iter()
        .find(|r| r.get("kaya_id") == Some(&CellValue::String("a2".into())))
        .expect("a2 row exists");
    assert_eq!(a2.get("grade"), Some(&CellValue::String("V4".into())), "grade updated to V4");
    assert_eq!(
        a2.get("date"),
        Some(&CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 9, 15).unwrap())),
        "date moved to 2026-09-15"
    );
}

#[test]
fn match_field_leaves_manual_rows_alone() {
    let store = temp_store("mf-manual");
    let view = climbing_view();
    // Hand-entered row, same day, no kaya_id.
    let mut manual = std::collections::BTreeMap::new();
    manual.insert(
        "date".to_string(),
        CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()),
    );
    manual.insert("climb_name".to_string(), CellValue::String("Project X".into()));
    store.create(&view, manual).unwrap();

    let res = ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    assert_eq!(res.created, 2, "manual row never matches; batch rows created fresh");
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|r| r.get("climb_name")
        == Some(&CellValue::String("Project X".into()))));
}

#[test]
fn match_field_record_without_key_is_skipped() {
    let store = temp_store("mf-nokey");
    let view = climbing_view();
    let b = batch(
        r#"{"source":"kaya","match_field":"kaya_id","records":[{"date":{"kind":"date","value":"2026-09-14"},"grade":{"kind":"string","value":"V1"}}]}"#,
    );
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.skipped, 1);
    assert!(store.list(&view, None).unwrap().is_empty());
}

#[test]
fn deleted_id_removes_source_created_untouched_row() {
    let store = temp_store("mf-del");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    let b = batch(r#"{"source":"kaya","match_field":"kaya_id","deleted_ids":["a2"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.deleted, 1);
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("kaya_id"), Some(&CellValue::String("a1".into())));
}

#[test]
fn deleted_id_with_user_edit_clears_source_fields_keeps_edit() {
    let store = temp_store("mf-del-edit");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    // User renames a2's climb.
    let mut row = store
        .list(&view, None)
        .unwrap()
        .into_iter()
        .find(|r| r.get("kaya_id") == Some(&CellValue::String("a2".into())))
        .unwrap();
    row.insert("climb_name".to_string(), CellValue::String("My Name".into()));
    store.update(&view, row).unwrap();

    let b = batch(r#"{"source":"kaya","match_field":"kaya_id","deleted_ids":["a2"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 1));
    let row = store
        .list(&view, None)
        .unwrap()
        .into_iter()
        .find(|r| r.get("climb_name") == Some(&CellValue::String("My Name".into())))
        .expect("edited row survives");
    assert_eq!(row.get("grade"), Some(&CellValue::Null), "source field cleared");
    assert_eq!(
        row.get("kaya_id"),
        Some(&CellValue::String("a2".into())),
        "identity fields exempt from clearing"
    );
}

#[test]
fn deleted_dates_are_ignored_in_match_field_mode() {
    let store = temp_store("mf-del-dates");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    let b = batch(
        r#"{"source":"kaya","match_field":"kaya_id","deleted_dates":["2026-09-14"]}"#,
    );
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 0));
    assert_eq!(store.list(&view, None).unwrap().len(), 2);
}

#[test]
fn unknown_match_field_errors_loudly() {
    let store = temp_store("mf-typo");
    let view = climbing_view();
    let b = batch(
        r#"{"source":"kaya","match_field":"kayaid","records":[{"kayaid":{"kind":"string","value":"a1"},"date":{"kind":"date","value":"2026-09-14"}}]}"#,
    );
    assert!(ingest(&store, &view, &b).is_err(), "typo'd match_field must not silently no-op");
}
