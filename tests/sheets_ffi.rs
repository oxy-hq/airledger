//! Non-network FFI tests for the sheets handle. Verifies the
//! pointer / lifecycle / JSON-shape contract without making real
//! HTTP calls.

use std::ffi::CString;

use airledger_engine::CellValue;

// We need to access the raw FFI symbols. They're #[no_mangle]
// extern "C" so we declare them here as the Dart side would.
extern "C" {
    fn airledger_engine_sheets_connect(
        default_spreadsheet_id_ptr: *const std::os::raw::c_char,
        service_account_json_ptr: *const std::os::raw::c_char,
        error_out: *mut *mut std::os::raw::c_char,
    ) -> *mut std::os::raw::c_void;

    fn airledger_engine_sheets_free_handle(
        handle: *mut std::os::raw::c_void,
    );

    fn airledger_engine_sheets_list(
        handle: *mut std::os::raw::c_void,
        view_json_ptr: *const std::os::raw::c_char,
        on_date_iso_ptr: *const std::os::raw::c_char,
    ) -> *mut std::os::raw::c_char;

    fn airledger_engine_free(ptr: *mut std::os::raw::c_char);
}

// A valid PKCS#8 RSA private key generated for testing only — does
// not authorize any real service. This is enough for the JWT signing
// path to succeed during construction so `sheets_connect` returns a
// non-null handle. The actual token exchange (which `list` would
// trigger) is what hits the network — we don't call any HTTP-touching
// op in this test.
const TEST_KEY: &str = "-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDH8RKWNoctMzSI
RPxoiMOg5BAg+P8UY81GkPCrNgLJ+l3yIDdL0/U6Krvx2EpegvDl54y/Pdyepg6T
sJh2LM3FAFRJEjbFCxQpAFiqMnB4G+T+Tcj4FfQfsBZIvOItDgaR/yJTYxLrxlvm
EpkRYbgZ2gQAezjbVm1Db0DhCnuvBOcyBhJ4F84tdfqVz0XwzwInfAa+9XmuapZG
0RDcZjy3JoR+v1AAg5J3Sd8GZ4yE1Y9pmaCpAXMUUopGyMHVrtwI3IZmTrTRoaSc
e1F+rW9o74WOFUKHM/JfMUlw/iqHwqlk3J2dCvbXJP+pIu9AvJjqgIyXxRbRGwh4
8SPLB5RpAgMBAAECggEAGo5bMRHfHwiu3qmavvCsbX2hp4Fp5/uMnvkVixu0wuyz
QrXcsSjp+xT9KGYsLBl3MOFb4Cy+Wjnxn6oryd5kg8DjVZTRYRR5g5MqYqpHFFNG
QtnTpEpFCO+W7H4Eaff1bIBjzcsBvbnqQZ+QXLF7Q+5Hu++X6q4hDoElYzWVDpRf
RyQ0CqkmW1DInhrn1uwiHo1QypJOmEKf/CZQu/CFbo9OWWywYWzo9HXDtGQRl/qP
WzKHQohB4Bnt0R66tHEZjUd/Rk/CvJBA7DI6PuKgvk8vSkdMpomEsuiH9pn7w/oA
W2v7eaQCBYIYFufZ19wYDfHvHb2T0lYtsTPlrDQpoQKBgQDvjkE/+kuoa3jrLNFt
nrqGUTGNHrwAGzZqAg2ynS+Lva8xbF7sUlrXXNwAfsfn5/k9SYJWZTH8VutehjLM
4BUcvJSkNwFFsbJ+iFNgWAlrEJxhDxX/MhqRsxz9KOJDPHQpf0EYZkrSpd4ZF8m4
1bRk9oqXMqQRy/T6qrYqMGZRmwKBgQDV57BBT8wEa9ix8ipYJN5JKa6+lQBcSe11
CtbLOGqJqExF8jSjPx7r8O44KEvFBQjBp1bAyJiekoOmcdEs2nXSlYZWMRfaIvLp
QV0VBKEx3SlGEh1Dl2JhMSlVDAmEZkPjA/A/MqYlS8/Lkc7CdjZZxQ5R9oJDIyXt
+L7nQVRbCwKBgGyhWmGv4Iq2W+4tMmlszSEQrAEoz4fxJ4o91/JxqXSMl5HFRiqA
HSdNXjotjOpL3T++ja+rNwbHWLAjVcekRgYR1qnXVvILxbVKbvVfuYbW6tjxg/dB
9CrCM1pmFvOLW5tlmJvUkVtbT1bXmu9zfQpZeOpz5PCKdsK/wHnLpQbXAoGAFIvI
SyhX8wbnzHK4uigjPnK4OB+3MEvr1qCXfBNoy0v9bRb8qf1lEdYjnsKKvNZNkPbe
NnDr0M3hmO0w+zhd14EwMd2yyrCmhKcsfZQTNxIvz5kuxEwMcGmLwf/IiKw9iZw4
2EsJ4XEAlWGFmM+ZByDbkn7nsfMmkOOpUOZl1m8CgYEAtxECc/2BXqGFt/LiK5Lh
KytIzABjuBbtjp42a8whqDpvZRabaXqOoT3DRKnBvAaQAYbHohE+lMmF2pSI5dKf
oJsXjUMK0wuNqxvkCSWvrcsOcyMMr6lUmU2BtfZBhxJWtMtGiTFxnTODBkn3JxXR
nUgPNBHm2vKnRfLrYWLNUcQ=
-----END PRIVATE KEY-----
";

fn make_sa_json() -> CString {
    let json = format!(
        r#"{{
            "type": "service_account",
            "project_id": "test-proj",
            "private_key_id": "test-kid",
            "private_key": {key:?},
            "client_email": "test@test-proj.iam.gserviceaccount.com",
            "client_id": "111111111111111111111",
            "token_uri": "https://oauth2.googleapis.com/token"
        }}"#,
        key = TEST_KEY,
    );
    CString::new(json).unwrap()
}

#[test]
fn cell_value_serializes_to_tagged_envelope() {
    let cases = [
        (CellValue::Null, r#"{"kind":"null"}"#),
        (CellValue::Bool(true), r#"{"kind":"bool","value":true}"#),
        (CellValue::Int(42), r#"{"kind":"int","value":42}"#),
        (
            CellValue::String("hi".into()),
            r#"{"kind":"string","value":"hi"}"#,
        ),
    ];
    for (val, expected) in cases {
        assert_eq!(serde_json::to_string(&val).unwrap(), expected);
    }
    // Float prints as a JSON number (`1.5` not `1`); we just check
    // round-trip rather than exact bytes since serde_json may emit
    // either `1.5` or `1.5000000000000000` depending on context.
    let f = CellValue::Float(1.5);
    let s = serde_json::to_string(&f).unwrap();
    let back: CellValue = serde_json::from_str(&s).unwrap();
    assert_eq!(f, back);

    // Date: ISO-8601, no time.
    let d = CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 6, 19).unwrap());
    assert_eq!(
        serde_json::to_string(&d).unwrap(),
        r#"{"kind":"date","value":"2026-06-19"}"#
    );
}

#[test]
fn connect_with_invalid_sa_returns_null_and_writes_error() {
    unsafe {
        let sid = CString::new("test-spreadsheet").unwrap();
        let bad = CString::new("{ not json").unwrap();
        let mut err: *mut std::os::raw::c_char = std::ptr::null_mut();
        let h = airledger_engine_sheets_connect(sid.as_ptr(), bad.as_ptr(), &mut err);
        assert!(h.is_null(), "handle should be null on bad input");
        assert!(!err.is_null(), "error out should be populated");
        let msg = std::ffi::CStr::from_ptr(err).to_str().unwrap().to_string();
        airledger_engine_free(err);
        assert!(
            msg.contains("service account json"),
            "expected message about JSON parse, got: {msg}"
        );
    }
}

#[test]
fn connect_then_free_roundtrips_a_valid_sa() {
    unsafe {
        let sid = CString::new("test-spreadsheet").unwrap();
        let sa = make_sa_json();
        let mut err: *mut std::os::raw::c_char = std::ptr::null_mut();
        let h = airledger_engine_sheets_connect(sid.as_ptr(), sa.as_ptr(), &mut err);
        assert!(!h.is_null(), "connect failed: {:?}", err);
        assert!(err.is_null(), "no error expected on valid sa");
        // No network call yet — list() with a malformed view should
        // return a JSON error without touching the wire.
        let view = CString::new("not a valid view json").unwrap();
        let result = airledger_engine_sheets_list(h, view.as_ptr(), std::ptr::null());
        assert!(!result.is_null());
        let msg = std::ffi::CStr::from_ptr(result).to_str().unwrap().to_string();
        airledger_engine_free(result);
        assert!(msg.contains("error"), "expected error JSON, got: {msg}");
        assert!(msg.contains("view json"), "expected view-json error, got: {msg}");
        airledger_engine_sheets_free_handle(h);
    }
}

#[test]
fn null_handle_calls_return_error_json() {
    unsafe {
        let view = CString::new(r#"{"name":"x","datasource":"gsheets","table":"x","dimensions":[]}"#).unwrap();
        let result = airledger_engine_sheets_list(
            std::ptr::null_mut(),
            view.as_ptr(),
            std::ptr::null(),
        );
        assert!(!result.is_null());
        let msg = std::ffi::CStr::from_ptr(result).to_str().unwrap().to_string();
        airledger_engine_free(result);
        assert!(msg.contains("null handle"), "got: {msg}");
    }
}
