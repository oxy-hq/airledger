//! End-to-end integration test for the sheets module. Skipped unless
//! both `AIRLEDGER_SHEETS_TEST_CREDS_PATH` and
//! `AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID` are set in the environment.
//!
//! What it does, against the live workbook:
//!   1. Defines an inline view + a temp tab name (`__airledger_test_<ts>`)
//!   2. ensure_sheet() — creates the tab + writes the header row
//!   3. create() — inserts one row, asserts __row = 0 returned
//!   4. list() — reads back the row, asserts the round-trip
//!   5. update() — modifies the row, asserts the read-back matches
//!   6. delete() — removes the row
//!   7. delete the tab via batchUpdate to leave the workbook clean
//!
//! Run:
//!   AIRLEDGER_SHEETS_TEST_CREDS_PATH=/path/to/sa.json \
//!     AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID=1abc...xyz \
//!     cargo test --test sheets_integration -- --nocapture

use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use airledger_engine::{
    parse_view, CellValue, Record, SheetsRepository, ROW_INDEX_KEY,
};

fn maybe_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|s| !s.is_empty())
}

const TEST_VIEW: &str = "name: test_round_trip
datasource: gsheets
table: REPLACED_BELOW
date_field: date
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: date, expr: date }
  - { name: note, type: string, expr: note }
  - { name: count, type: number, expr: count }
";

#[test]
fn round_trip_against_live_workbook() {
    let Some(creds_path) = maybe_env("AIRLEDGER_SHEETS_TEST_CREDS_PATH") else {
        eprintln!("SKIP: AIRLEDGER_SHEETS_TEST_CREDS_PATH not set");
        return;
    };
    let Some(spreadsheet_id) = maybe_env("AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID") else {
        eprintln!("SKIP: AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID not set");
        return;
    };

    let sa_json = std::fs::read_to_string(&creds_path)
        .unwrap_or_else(|e| panic!("read creds at {creds_path}: {e}"));

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let tab = format!("__airledger_test_{ts}");
    let view_yaml = TEST_VIEW.replace("REPLACED_BELOW", &tab);
    let view = parse_view(&view_yaml).expect("parse view");

    let repo = SheetsRepository::new(spreadsheet_id, &sa_json).expect("new repo");

    repo.ensure_sheet(&view).expect("ensure_sheet");

    let mut record = Record::new();
    record.insert("date".into(), CellValue::Date(
        chrono::NaiveDate::from_ymd_opt(2026, 6, 19).unwrap()
    ));
    record.insert("note".into(), CellValue::String("hello rust".into()));
    record.insert("count".into(), CellValue::Int(42));

    let created = repo.create(&view, record).expect("create");
    assert!(matches!(created.get(ROW_INDEX_KEY), Some(CellValue::Int(0))));
    assert!(created.get("id").is_some(), "id auto-assigned");

    let rows = repo.list(&view, None).expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("note"),
        Some(&CellValue::String("hello rust".into()))
    );
    assert_eq!(rows[0].get("count"), Some(&CellValue::Int(42)));

    let mut updated = rows[0].clone();
    updated.insert("note".into(), CellValue::String("updated".into()));
    repo.update(&view, updated).expect("update");

    let rows = repo.list(&view, None).expect("list-2");
    assert_eq!(
        rows[0].get("note"),
        Some(&CellValue::String("updated".into()))
    );

    repo.delete(&view, &rows[0]).expect("delete");
    let rows = repo.list(&view, None).expect("list-3");
    assert_eq!(rows.len(), 0, "row should be gone after delete");

    // Best-effort tab cleanup (not part of the assertion contract).
    // We resolve the sheet id and ask the API to drop it.
    eprintln!("test tab: {tab} (leaving in place; delete manually if you want it gone)");
}
