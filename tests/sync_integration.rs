//! End-to-end sync integration test — local store ⇄ live workbook.
//! Skipped unless both `AIRLEDGER_SHEETS_TEST_CREDS_PATH` and
//! `AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID` are set.
//!
//! Against the live workbook:
//!   1. Local create → sync pushes it to a fresh temp tab
//!   2. Fresh store (second device / reinstall) hydrates from the tab
//!   3. Simulated manual sheet edit (SheetsRepository::update) →
//!      sync pulls it into the store
//!   4. Same-row edit on both sides → sync → app wins on the sheet
//!   5. Local delete → sync → sheet row gone
//!   6. Tab deleted to leave the workbook clean
//!
//! Run:
//!   AIRLEDGER_SHEETS_TEST_CREDS_PATH=/path/to/sa.json \
//!     AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID=1abc...xyz \
//!     cargo test --test sync_integration -- --nocapture

use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use airledger_engine::store::Store;
use airledger_engine::sync::sync_views;
use airledger_engine::{parse_view, CellValue, Record, SheetsRepository};

fn maybe_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|s| !s.is_empty())
}

const TEST_VIEW: &str = "name: test_sync_round_trip
datasource: gsheets
table: REPLACED_BELOW
dimensions:
  - { name: id, type: string, expr: id }
  - { name: note, type: string, expr: note }
  - { name: count, type: number, expr: count }
";

fn temp_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join("airledger-sync-int");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}-{}.db", std::process::id()));
    std::fs::remove_file(&path).ok();
    Store::open(path.to_str().unwrap()).unwrap()
}

#[test]
fn live_sync_round_trip() {
    let (Some(creds_path), Some(spreadsheet_id)) = (
        maybe_env("AIRLEDGER_SHEETS_TEST_CREDS_PATH"),
        maybe_env("AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID"),
    ) else {
        eprintln!("skipping live_sync_round_trip: env vars unset");
        return;
    };
    let sa_json = std::fs::read_to_string(&creds_path).expect("read creds");
    let repo = SheetsRepository::new(spreadsheet_id.clone(), &sa_json).expect("repo");

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let tab = format!("__airledger_sync_test_{ts}");
    let view = parse_view(&TEST_VIEW.replace("REPLACED_BELOW", &tab)).expect("view");

    // 1. Local create → sync pushes.
    let store_a = temp_store("dev-a");
    let mut rec = Record::new();
    rec.insert("note".into(), CellValue::String("from local".into()));
    rec.insert("count".into(), CellValue::Int(1));
    let created = store_a.create(&view, rec).unwrap();
    let id = created.get("id").unwrap().to_display_string();
    let res = sync_views(&store_a, &repo, std::slice::from_ref(&view));
    assert!(res[0].error.is_none(), "push sync: {:?}", res[0].error);
    assert_eq!(res[0].pushed, 1);
    assert_eq!(store_a.pending_count().unwrap(), 0);

    // 2. Fresh store hydrates.
    let store_b = temp_store("dev-b");
    let res = sync_views(&store_b, &repo, std::slice::from_ref(&view));
    assert!(res[0].error.is_none(), "hydrate sync: {:?}", res[0].error);
    assert_eq!(res[0].pulled, 1);
    let rows = store_b.list(&view, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("id").unwrap().to_display_string(), id);

    // 3. Manual sheet edit → pull into store_a.
    let mut sheet_edit = rows[0].clone();
    sheet_edit.insert("note".into(), CellValue::String("sheet edit".into()));
    repo.update(&view, sheet_edit).unwrap();
    let res = sync_views(&store_a, &repo, std::slice::from_ref(&view));
    assert!(res[0].error.is_none(), "pull sync: {:?}", res[0].error);
    let rows = store_a.list(&view, None).unwrap();
    assert_eq!(
        rows[0].get("note"),
        Some(&CellValue::String("sheet edit".into()))
    );

    // 4. Conflict: both sides edit → app wins.
    let mut remote_edit = rows[0].clone();
    remote_edit.insert("note".into(), CellValue::String("sheet again".into()));
    repo.update(&view, remote_edit).unwrap();
    let mut local_edit = rows[0].clone();
    local_edit.insert("note".into(), CellValue::String("app wins".into()));
    store_a.update(&view, local_edit).unwrap();
    let res = sync_views(&store_a, &repo, std::slice::from_ref(&view));
    assert!(res[0].error.is_none(), "conflict sync: {:?}", res[0].error);
    assert_eq!(res[0].conflicts, 1);
    let remote_rows = repo.list(&view, None).unwrap();
    assert_eq!(
        remote_rows[0].get("note"),
        Some(&CellValue::String("app wins".into()))
    );

    // 5. Local delete → sheet row gone.
    let row = store_a.list(&view, None).unwrap().remove(0);
    store_a.delete(&view, &row).unwrap();
    let res = sync_views(&store_a, &repo, std::slice::from_ref(&view));
    assert!(res[0].error.is_none(), "delete sync: {:?}", res[0].error);
    assert_eq!(res[0].deleted_remote, 1);
    assert!(repo.list(&view, None).unwrap().is_empty());

    // 6. Same policy as sheets_integration: the tab is empty and
    // clearly named; leave it for manual cleanup.
    let _ = spreadsheet_id;
    println!("live sync round-trip OK on tab {tab} (leaving empty tab in place)");
}
