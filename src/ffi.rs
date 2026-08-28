//! C-ABI surface for the engine. Mirrors airlayer's
//! `lib/airlayer_ffi.rs` pattern: every exposed function takes nul-
//! terminated UTF-8 strings in, returns malloc'd nul-terminated UTF-8
//! strings out, with a separate `_free` function so the calling
//! language can release them safely.
//!
//! Complex types travel as JSON. That keeps the ABI flat (no need to
//! describe a Dart struct that matches a Rust struct byte-for-byte) at
//! the cost of one serialize + one parse per call. For the
//! schema-shaped types the engine handles, the JSON layer is well
//! below 1ms overhead — fine for the form's interactivity budget.
//!
//! Error reporting: parse / overlay errors come back as a JSON object
//! `{"error": "..."}` so the Dart side can pattern-match without
//! needing a separate out-param. Successful calls return the result
//! directly serialized (no wrapper).

use std::ffi::{c_char, CStr, CString};
use std::sync::Mutex;

use chrono::NaiveDate;
use serde::Serialize;

use crate::sheets::SheetsRepository;
use crate::value::Record;
use crate::ViewSchema;
use crate::{apply_overlay, parse_input_overlay, parse_view};

/// Returns the engine version. Stable identity string the Dart side
/// can `assert!` to verify FFI plumbing is alive before calling
/// anything more interesting.
#[no_mangle]
pub extern "C" fn airledger_engine_version() -> *mut c_char {
    let s = format!("airledger-engine {}", env!("CARGO_PKG_VERSION"));
    string_to_ptr(s)
}

/// Parse a `.view.yml` document. Returns JSON-serialized
/// [`crate::ViewSchema`] on success, or `{"error": "..."}` on failure.
///
/// # Safety
/// `yaml_ptr` must be a valid pointer to a nul-terminated UTF-8
/// string. Caller owns the returned pointer and MUST release it via
/// [`airledger_engine_free`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_parse_view(
    yaml_ptr: *const c_char,
) -> *mut c_char {
    let yaml = match unsafe { c_str_to_str(yaml_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    match parse_view(yaml) {
        Ok(view) => result_json(&view),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Parse a `.input.yml` document. Returns JSON-serialized
/// `InputOverlay` on success, or `{"error": "..."}` on failure.
///
/// # Safety
/// As [`airledger_engine_parse_view`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_parse_input_overlay(
    yaml_ptr: *const c_char,
) -> *mut c_char {
    let yaml = match unsafe { c_str_to_str(yaml_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    match parse_input_overlay(yaml) {
        Ok(overlay) => result_json(&FfiInputOverlay::from(&overlay)),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Parse both files and merge — the canonical entry point the Dart
/// side will use. Saves a round-trip through Dart-side glue code.
///
/// # Safety
/// Both pointers as [`airledger_engine_parse_view`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_parse_view_pair(
    view_yaml_ptr: *const c_char,
    input_yaml_ptr: *const c_char,
) -> *mut c_char {
    let view_yaml = match unsafe { c_str_to_str(view_yaml_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    let input_yaml = match unsafe { c_str_to_str(input_yaml_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    let view = match parse_view(view_yaml) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("view parse: {e}")),
    };
    let overlay = match parse_input_overlay(input_yaml) {
        Ok(o) => o,
        Err(e) => return error_json(&format!("input parse: {e}")),
    };
    match apply_overlay(view, overlay) {
        Ok(merged) => result_json(&merged),
        Err(e) => error_json(&format!("merge: {e}")),
    }
}

/// Release a string previously returned by any
/// `airledger_engine_*` function. No-op on null. Calling on a pointer
/// that wasn't produced by this crate is undefined behavior.
///
/// # Safety
/// `ptr` must have been returned by one of the engine's `extern "C"`
/// functions, or be null. Must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    // Reclaim the CString that owns this allocation. It drops at end
    // of scope and frees the buffer.
    drop(unsafe { CString::from_raw(ptr) });
}

// ----------------------------------------------------------- helpers

unsafe fn c_str_to_str<'a>(ptr: *const c_char) -> Result<&'a str, String> {
    if ptr.is_null() {
        return Err("null pointer".into());
    }
    let cstr = unsafe { CStr::from_ptr(ptr) };
    cstr.to_str().map_err(|e| format!("invalid utf-8: {e}"))
}

fn string_to_ptr(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("error: interior nul").unwrap())
        .into_raw()
}

fn result_json<T: Serialize>(value: &T) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(s) => string_to_ptr(s),
        Err(e) => error_json(&format!("serialize: {e}")),
    }
}

fn error_json(msg: &str) -> *mut c_char {
    // Use serde_json to build the object so every JSON-illegal char
    // (control chars, embedded quotes, etc.) is escaped properly.
    // The previous hand-rolled escape only handled `\` and `"`, so a
    // 401 body containing a JSON sub-document with newlines broke the
    // outer parse on the Dart side with "Control character in string".
    let s = serde_json::json!({ "error": msg }).to_string();
    string_to_ptr(s)
}

// ------------------------------------------------ FFI projection types
//
// InputOverlay contains a BTreeMap<String, DimensionOverlay> + a
// BTreeSet<String> in groups. serde_json can serialize both, but we
// project to a flat structure so the JSON keys match what the Dart
// side would expect from the YAML directly (lower friction on the
// consumer side).

#[derive(Serialize)]
struct FfiInputOverlay<'a> {
    view_name: &'a str,
    date_field: Option<&'a str>,
    spreadsheet_id: Option<&'a str>,
    icon: Option<&'a str>,
    top_metric: Option<&'a str>,
    has_input_overlay: bool,
    plannable: Option<&'a crate::schema::input::Plannable>,
    list_display: Option<&'a crate::schema::input::ListDisplay>,
    post_log: Option<&'a crate::schema::input::PostLog>,
    repeat_group: Option<&'a crate::schema::input::RepeatGroup>,
    dimensions: &'a std::collections::BTreeMap<
        String,
        crate::schema::overlay::DimensionOverlay,
    >,
    groups: &'a std::collections::BTreeMap<
        String,
        std::collections::BTreeSet<String>,
    >,
}

impl<'a> From<&'a crate::schema::overlay::InputOverlay> for FfiInputOverlay<'a> {
    fn from(o: &'a crate::schema::overlay::InputOverlay) -> Self {
        Self {
            view_name: &o.view_name,
            date_field: o.date_field.as_deref(),
            spreadsheet_id: o.spreadsheet_id.as_deref(),
            icon: o.icon.as_deref(),
            top_metric: o.top_metric.as_deref(),
            has_input_overlay: true,
            plannable: o.plannable.as_ref(),
            list_display: o.list_display.as_ref(),
            post_log: o.post_log.as_ref(),
            repeat_group: o.repeat_group.as_ref(),
            dimensions: &o.dimensions,
            groups: &o.groups,
        }
    }
}

// =========================================================== sheets handle
//
// The sheets module is stateful (token cache, header cache, HTTP client).
// We expose it as an opaque handle the Dart side holds across calls.
// `SheetsRepository` itself is `!Sync` because it uses `RefCell` for the
// caches, so we wrap it in `Mutex` to make the raw pointer safe to share
// across threads in case Dart's isolate model ever needs it.

/// Opaque handle the Dart side holds onto. Created by
/// [`airledger_engine_sheets_connect`], freed by
/// [`airledger_engine_sheets_free_handle`].
pub struct SheetsHandle(Mutex<SheetsRepository>);

/// Build a sheets repository. Returns NULL on failure; the heap-
/// allocated error message goes through `error_out` (caller frees it
/// via [`airledger_engine_free`]). Pass NULL for `error_out` to
/// suppress error reporting.
///
/// # Safety
/// `default_spreadsheet_id_ptr` and `service_account_json_ptr` must
/// be valid nul-terminated UTF-8. `error_out` must be either NULL or
/// a valid writable `*mut *mut c_char`.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_sheets_connect(
    default_spreadsheet_id_ptr: *const c_char,
    service_account_json_ptr: *const c_char,
    error_out: *mut *mut c_char,
) -> *mut SheetsHandle {
    let result = (|| -> Result<SheetsRepository, String> {
        let sid = unsafe { c_str_to_str(default_spreadsheet_id_ptr) }?;
        let sa = unsafe { c_str_to_str(service_account_json_ptr) }?;
        SheetsRepository::new(sid.to_string(), sa).map_err(|e| e.to_string())
    })();
    match result {
        Ok(repo) => {
            if !error_out.is_null() {
                unsafe { *error_out = std::ptr::null_mut() };
            }
            Box::into_raw(Box::new(SheetsHandle(Mutex::new(repo))))
        }
        Err(e) => {
            if !error_out.is_null() {
                unsafe { *error_out = string_to_ptr(e) };
            }
            std::ptr::null_mut()
        }
    }
}

/// Drop a handle from [`airledger_engine_sheets_connect`]. No-op on
/// NULL.
///
/// # Safety
/// Must be called at most once per handle, and the handle must not
/// be used after this call.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_sheets_free_handle(
    handle: *mut SheetsHandle,
) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// `ensure_sheet` over FFI. Returns `{"ok":true}` on success or
/// `{"error":"..."}` on failure.
///
/// # Safety
/// `handle` must be valid (from `_connect`). `view_json_ptr` must be
/// a nul-terminated UTF-8 JSON [`ViewSchema`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_sheets_ensure(
    handle: *mut SheetsHandle,
    view_json_ptr: *const c_char,
) -> *mut c_char {
    sheets_call(handle, view_json_ptr, |repo, view| {
        repo.ensure_sheet(view).map(|()| serde_json::json!({ "ok": true }))
    })
}

/// `list` over FFI. `on_date_iso_ptr` may be NULL (no date filter) or
/// `YYYY-MM-DD`. Returns a JSON array of records on success.
///
/// # Safety
/// As [`airledger_engine_sheets_ensure`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_sheets_list(
    handle: *mut SheetsHandle,
    view_json_ptr: *const c_char,
    on_date_iso_ptr: *const c_char,
) -> *mut c_char {
    let on_date = if on_date_iso_ptr.is_null() {
        None
    } else {
        match unsafe { c_str_to_str(on_date_iso_ptr) } {
            Ok(s) => match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(e) => return error_json(&format!("on_date parse: {e}")),
            },
            Err(e) => return error_json(&e),
        }
    };
    sheets_call(handle, view_json_ptr, |repo, view| {
        repo.list(view, on_date)
    })
}

/// `create` over FFI. `record_json_ptr` is a JSON object with
/// dimension-name keys and tagged `CellValue` values (see
/// [`crate::value::CellValue`]'s serialization shape). Returns the
/// inserted record (including any auto-assigned `id` and `__row = 0`).
///
/// # Safety
/// As [`airledger_engine_sheets_ensure`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_sheets_create(
    handle: *mut SheetsHandle,
    view_json_ptr: *const c_char,
    record_json_ptr: *const c_char,
) -> *mut c_char {
    let record = match parse_record_json(record_json_ptr) {
        Ok(r) => r,
        Err(e) => return error_json(&e),
    };
    sheets_call(handle, view_json_ptr, |repo, view| {
        repo.create(view, record)
    })
}

/// `update` over FFI. Same record shape as
/// [`airledger_engine_sheets_create`]. Returns `{"ok":true}`.
///
/// # Safety
/// As [`airledger_engine_sheets_ensure`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_sheets_update(
    handle: *mut SheetsHandle,
    view_json_ptr: *const c_char,
    record_json_ptr: *const c_char,
) -> *mut c_char {
    let record = match parse_record_json(record_json_ptr) {
        Ok(r) => r,
        Err(e) => return error_json(&e),
    };
    sheets_call(handle, view_json_ptr, |repo, view| {
        repo.update(view, record).map(|()| serde_json::json!({ "ok": true }))
    })
}

/// `delete` over FFI. Same record shape as
/// [`airledger_engine_sheets_create`]. Returns `{"ok":true}`.
///
/// # Safety
/// As [`airledger_engine_sheets_ensure`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_sheets_delete(
    handle: *mut SheetsHandle,
    view_json_ptr: *const c_char,
    record_json_ptr: *const c_char,
) -> *mut c_char {
    let record = match parse_record_json(record_json_ptr) {
        Ok(r) => r,
        Err(e) => return error_json(&e),
    };
    sheets_call(handle, view_json_ptr, |repo, view| {
        repo.delete(view, &record).map(|()| serde_json::json!({ "ok": true }))
    })
}

// ----------------------------------------------------- sheets helpers

fn sheets_call<F, R>(
    handle: *mut SheetsHandle,
    view_json_ptr: *const c_char,
    f: F,
) -> *mut c_char
where
    R: Serialize,
    F: FnOnce(&SheetsRepository, &ViewSchema) -> Result<R, crate::sheets::SheetsError>,
{
    if handle.is_null() {
        return error_json("null handle");
    }
    let view_json = match unsafe { c_str_to_str(view_json_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    let view: ViewSchema = match serde_json::from_str(view_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("view json: {e}")),
    };
    let handle = unsafe { &*handle };
    let repo = match handle.0.lock() {
        Ok(g) => g,
        Err(_) => return error_json("handle mutex poisoned"),
    };
    match f(&repo, &view) {
        Ok(value) => result_json(&value),
        Err(e) => error_json(&e.to_string()),
    }
}

fn parse_record_json(ptr: *const c_char) -> Result<Record, String> {
    let json = unsafe { c_str_to_str(ptr) }?;
    serde_json::from_str::<Record>(json).map_err(|e| format!("record json: {e}"))
}

// `CellValue` already implements Serialize/Deserialize with a tagged
// envelope (`{"kind":"int","value":42}`). `Record = BTreeMap<String,
// CellValue>` serializes to a flat object where each value is one of
// those envelopes — that's the exact shape the Dart side will use.
//
// We re-export the type alias here so consumers reading the FFI source
// can find the wire shape in one place.
pub use crate::value::CellValue as FfiCellValue;

// ========================================================== ledger handle
//
// Local-first store + sync. Mirrors the sheets handle pattern: the
// store and the sheets repo live behind one opaque handle; local CRUD
// never touches the network, sync does.

use crate::store::{Store, StoreError};
use crate::sync::sync_views;

pub struct LedgerHandle {
    store: Mutex<Store>,
    sheets: Mutex<SheetsRepository>,
    /// Kept so sync can open its own SQLite connection — WAL lets the
    /// UI's CRUD connection proceed while a sync is mid-flight, so a
    /// save never waits on a running sync.
    db_path: String,
}

/// Open the local store at `db_path` and prepare the sheets repo for
/// sync. No network here — credentials are parsed, not exercised.
/// Returns NULL on failure with the message in `error_out`.
///
/// # Safety
/// String pointers must be valid nul-terminated UTF-8. `error_out`
/// must be NULL or a valid writable `*mut *mut c_char`.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_open(
    db_path_ptr: *const c_char,
    default_spreadsheet_id_ptr: *const c_char,
    service_account_json_ptr: *const c_char,
    error_out: *mut *mut c_char,
) -> *mut LedgerHandle {
    let result = (|| -> Result<LedgerHandle, String> {
        let db_path = unsafe { c_str_to_str(db_path_ptr) }?;
        let sid = unsafe { c_str_to_str(default_spreadsheet_id_ptr) }?;
        let sa = unsafe { c_str_to_str(service_account_json_ptr) }?;
        let store = Store::open(db_path).map_err(|e| e.to_string())?;
        let sheets =
            SheetsRepository::new(sid.to_string(), sa).map_err(|e| e.to_string())?;
        Ok(LedgerHandle {
            store: Mutex::new(store),
            sheets: Mutex::new(sheets),
            db_path: db_path.to_string(),
        })
    })();
    match result {
        Ok(h) => {
            if !error_out.is_null() {
                unsafe { *error_out = std::ptr::null_mut() };
            }
            Box::into_raw(Box::new(h))
        }
        Err(e) => {
            if !error_out.is_null() {
                unsafe { *error_out = string_to_ptr(e) };
            }
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// At most once per handle; handle unusable afterwards.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_free_handle(handle: *mut LedgerHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Local `list` — no network. `on_date_iso_ptr` may be NULL or
/// `YYYY-MM-DD`. Returns a JSON array of tagged-envelope records.
///
/// # Safety
/// `handle` from `_ledger_open`; strings nul-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_list(
    handle: *mut LedgerHandle,
    view_json_ptr: *const c_char,
    on_date_iso_ptr: *const c_char,
) -> *mut c_char {
    let on_date = if on_date_iso_ptr.is_null() {
        None
    } else {
        match unsafe { c_str_to_str(on_date_iso_ptr) } {
            Ok(s) => match NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(e) => return error_json(&format!("on_date parse: {e}")),
            },
            Err(e) => return error_json(&e),
        }
    };
    ledger_call(handle, view_json_ptr, |store, view| store.list(view, on_date))
}

/// Local `create` — same record envelope as the sheets FFI. Returns
/// the stored record (with any auto-assigned `id`).
///
/// # Safety
/// As [`airledger_engine_ledger_list`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_create(
    handle: *mut LedgerHandle,
    view_json_ptr: *const c_char,
    record_json_ptr: *const c_char,
) -> *mut c_char {
    let record = match parse_record_json(record_json_ptr) {
        Ok(r) => r,
        Err(e) => return error_json(&e),
    };
    ledger_call(handle, view_json_ptr, |store, view| store.create(view, record))
}

/// Local `update` — addresses by `id`. Returns `{"ok":true}`.
///
/// # Safety
/// As [`airledger_engine_ledger_list`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_update(
    handle: *mut LedgerHandle,
    view_json_ptr: *const c_char,
    record_json_ptr: *const c_char,
) -> *mut c_char {
    let record = match parse_record_json(record_json_ptr) {
        Ok(r) => r,
        Err(e) => return error_json(&e),
    };
    ledger_call(handle, view_json_ptr, |store, view| {
        store
            .update(view, record)
            .map(|()| serde_json::json!({ "ok": true }))
    })
}

/// Local `delete` — tombstones synced rows, removes unsynced ones.
/// Returns `{"ok":true}`.
///
/// # Safety
/// As [`airledger_engine_ledger_list`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_delete(
    handle: *mut LedgerHandle,
    view_json_ptr: *const c_char,
    record_json_ptr: *const c_char,
) -> *mut c_char {
    let record = match parse_record_json(record_json_ptr) {
        Ok(r) => r,
        Err(e) => return error_json(&e),
    };
    ledger_call(handle, view_json_ptr, |store, view| {
        store
            .delete(view, &record)
            .map(|()| serde_json::json!({ "ok": true }))
    })
}

/// Pending (un-pushed) change count: `{"pending": N}`.
///
/// # Safety
/// `handle` must be a valid ledger handle.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_pending(
    handle: *mut LedgerHandle,
) -> *mut c_char {
    if handle.is_null() {
        return error_json("null handle");
    }
    let handle = unsafe { &*handle };
    let store = match handle.store.lock() {
        Ok(g) => g,
        Err(_) => return error_json("store mutex poisoned"),
    };
    match store.pending_count() {
        Ok(n) => result_json(&serde_json::json!({ "pending": n })),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Run a full sync for every view in `views_json_ptr` (a JSON array
/// of ViewSchema). Returns the JSON array of per-view results —
/// individual view failures land in each result's `error` field, so
/// this call only returns `{"error": ...}` for input-shape problems.
///
/// # Safety
/// `handle` valid; `views_json_ptr` nul-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_sync(
    handle: *mut LedgerHandle,
    views_json_ptr: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return error_json("null handle");
    }
    let views_json = match unsafe { c_str_to_str(views_json_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    let views: Vec<ViewSchema> = match serde_json::from_str(views_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("views json: {e}")),
    };
    let handle = unsafe { &*handle };
    // Own connection for the sync — the UI's store mutex stays free,
    // so create/list during a sync run at full speed (WAL handles
    // the write interleaving; busy_timeout covers the brief overlap).
    let sync_store = match Store::open(&handle.db_path) {
        Ok(s) => s,
        Err(e) => return error_json(&e.to_string()),
    };
    let sheets = match handle.sheets.lock() {
        Ok(g) => g,
        Err(_) => return error_json("handle mutex poisoned"),
    };
    result_json(&sync_views(&sync_store, &*sheets, &views))
}

// ----------------------------------------------------- ledger helpers

fn ledger_call<F, R>(
    handle: *mut LedgerHandle,
    view_json_ptr: *const c_char,
    f: F,
) -> *mut c_char
where
    R: Serialize,
    F: FnOnce(&Store, &ViewSchema) -> Result<R, StoreError>,
{
    if handle.is_null() {
        return error_json("null handle");
    }
    let view_json = match unsafe { c_str_to_str(view_json_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    let view: ViewSchema = match serde_json::from_str(view_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("view json: {e}")),
    };
    let handle = unsafe { &*handle };
    let store = match handle.store.lock() {
        Ok(g) => g,
        Err(_) => return error_json("store mutex poisoned"),
    };
    match f(&store, &view) {
        Ok(value) => result_json(&value),
        Err(e) => error_json(&e.to_string()),
    }
}
