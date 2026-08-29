//! Non-network FFI tests for the ledger handle: open a store-backed
//! ledger, CRUD locally, check the pending count — all offline.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

// Reference the crate so the rlib (and its #[no_mangle] symbols)
// actually links into this test binary.
#[allow(unused_imports)]
use airledger_engine::CellValue;

extern "C" {
    fn airledger_engine_ledger_open(
        db_path_ptr: *const c_char,
        default_spreadsheet_id_ptr: *const c_char,
        service_account_json_ptr: *const c_char,
        error_out: *mut *mut c_char,
    ) -> *mut c_void;

    fn airledger_engine_ledger_free_handle(handle: *mut c_void);

    fn airledger_engine_ledger_list(
        handle: *mut c_void,
        view_json_ptr: *const c_char,
        on_date_iso_ptr: *const c_char,
    ) -> *mut c_char;

    fn airledger_engine_ledger_create(
        handle: *mut c_void,
        view_json_ptr: *const c_char,
        record_json_ptr: *const c_char,
    ) -> *mut c_char;

    fn airledger_engine_ledger_delete(
        handle: *mut c_void,
        view_json_ptr: *const c_char,
        record_json_ptr: *const c_char,
    ) -> *mut c_char;

    fn airledger_engine_ledger_pending(handle: *mut c_void) -> *mut c_char;

    fn airledger_engine_ledger_ingest(
        handle: *mut c_void,
        view_json_ptr: *const c_char,
        batch_json_ptr: *const c_char,
    ) -> *mut c_char;

    fn airledger_engine_ledger_meta_get(
        handle: *mut c_void,
        key_ptr: *const c_char,
    ) -> *mut c_char;

    fn airledger_engine_ledger_meta_set(
        handle: *mut c_void,
        key_ptr: *const c_char,
        value_ptr: *const c_char,
    ) -> *mut c_char;

    fn airledger_engine_free(ptr: *mut c_char);
}

// Structure-only SA json — the PEM is validated lazily at token
// time, which offline CRUD never reaches.
const FAKE_SA: &str = r#"{
    "type": "service_account",
    "project_id": "t",
    "private_key_id": "k1",
    "private_key": "-----BEGIN PRIVATE KEY-----\nZmFrZQ==\n-----END PRIVATE KEY-----\n",
    "client_email": "t@t.iam.gserviceaccount.com",
    "client_id": "1",
    "token_uri": "https://oauth2.googleapis.com/token"
}"#;

const VIEW_JSON: &str = r#"{
    "name": "weight",
    "datasource": "gsheets",
    "table": "weight",
    "dimensions": [
        { "name": "id", "type": "string", "expr": "id" },
        { "name": "weight_lbs", "type": "number", "expr": "weight_lbs" }
    ]
}"#;

unsafe fn take_string(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null());
    let s = CStr::from_ptr(ptr).to_str().unwrap().to_string();
    airledger_engine_free(ptr);
    s
}

#[test]
fn ledger_crud_offline_round_trip() {
    let dir = std::env::temp_dir().join("airledger-ledger-ffi");
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join(format!("ffi-{}.db", std::process::id()));
    std::fs::remove_file(&db).ok();

    unsafe {
        let db_path = CString::new(db.to_str().unwrap()).unwrap();
        let sid = CString::new("unused-spreadsheet").unwrap();
        let sa = CString::new(FAKE_SA).unwrap();
        let view = CString::new(VIEW_JSON).unwrap();
        let mut err: *mut c_char = std::ptr::null_mut();

        let h = airledger_engine_ledger_open(db_path.as_ptr(), sid.as_ptr(), sa.as_ptr(), &mut err);
        assert!(!h.is_null(), "open failed: {:?}", err);

        // create
        let record = CString::new(r#"{"weight_lbs":{"kind":"float","value":180.5}}"#).unwrap();
        let created = take_string(airledger_engine_ledger_create(
            h,
            view.as_ptr(),
            record.as_ptr(),
        ));
        let created_json: serde_json::Value = serde_json::from_str(&created).unwrap();
        assert!(created_json["error"].is_null(), "create error: {created}");
        assert_eq!(created_json["id"]["kind"], "string", "id auto-assigned");

        // list
        let listed = take_string(airledger_engine_ledger_list(
            h,
            view.as_ptr(),
            std::ptr::null(),
        ));
        let listed_json: serde_json::Value = serde_json::from_str(&listed).unwrap();
        assert_eq!(listed_json.as_array().unwrap().len(), 1);

        // pending
        let pending = take_string(airledger_engine_ledger_pending(h));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&pending).unwrap()["pending"],
            1
        );

        // delete (never synced → removed outright, pending back to 0)
        let created_c = CString::new(created).unwrap();
        let deleted = take_string(airledger_engine_ledger_delete(
            h,
            view.as_ptr(),
            created_c.as_ptr(),
        ));
        assert!(deleted.contains("ok"), "delete: {deleted}");
        let pending = take_string(airledger_engine_ledger_pending(h));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&pending).unwrap()["pending"],
            0
        );

        airledger_engine_ledger_free_handle(h);
    }
    std::fs::remove_file(&db).ok();
}

#[test]
fn ledger_null_handle_returns_error_json() {
    unsafe {
        let out = take_string(airledger_engine_ledger_pending(std::ptr::null_mut()));
        assert!(out.contains("error"));
    }
}

#[test]
fn ledger_ingest_and_meta_round_trip() {
    let dir = std::env::temp_dir().join("airledger-ledger-ffi");
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join(format!("ingest-{}.db", std::process::id()));
    std::fs::remove_file(&db).ok();
    unsafe {
        let db_path = CString::new(db.to_str().unwrap()).unwrap();
        let sid = CString::new("unused").unwrap();
        let sa = CString::new(FAKE_SA).unwrap();
        let mut err: *mut c_char = std::ptr::null_mut();
        let h = airledger_engine_ledger_open(db_path.as_ptr(), sid.as_ptr(), sa.as_ptr(), &mut err);
        assert!(!h.is_null());

        let view = CString::new(
            r#"{
            "name":"weight","datasource":"gsheets","table":"weight","date_field":"date",
            "dimensions":[
                {"name":"id","type":"string","expr":"id"},
                {"name":"date","type":"date","expr":"date"},
                {"name":"body_fat_withing","type":"number","expr":"body_fat_withing"}
            ]}"#,
        )
        .unwrap();
        let batch = CString::new(
            r#"{
            "source":"withings","owned_fields":["body_fat_withing"],
            "records":[{"date":{"kind":"date","value":"2026-08-28"},
                        "body_fat_withing":{"kind":"float","value":18.2}}]}"#,
        )
        .unwrap();
        let out = take_string(airledger_engine_ledger_ingest(h, view.as_ptr(), batch.as_ptr()));
        let res: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(res["created"], 1, "ingest: {out}");

        let key = CString::new("integration_cursor_withings").unwrap();
        let val = CString::new("1756400000").unwrap();
        let out = take_string(airledger_engine_ledger_meta_set(h, key.as_ptr(), val.as_ptr()));
        assert!(out.contains("ok"));
        let out = take_string(airledger_engine_ledger_meta_get(h, key.as_ptr()));
        let res: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(res["value"], "1756400000");

        airledger_engine_ledger_free_handle(h);
    }
    std::fs::remove_file(&db).ok();
}
