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

use serde::Serialize;

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
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    string_to_ptr(format!("{{\"error\":\"{escaped}\"}}"))
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
