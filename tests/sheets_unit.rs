//! Non-network tests for the sheets module. The full round-trip
//! lives in `sheets_integration.rs` behind an env gate.

use airledger_engine::ServiceAccount;

#[test]
fn service_account_parses_minimal_json() {
    let json = r#"{
        "type": "service_account",
        "project_id": "test-proj",
        "private_key_id": "abc123",
        "private_key": "-----BEGIN PRIVATE KEY-----\nfake\n-----END PRIVATE KEY-----\n",
        "client_email": "test@test-proj.iam.gserviceaccount.com",
        "client_id": "100000000000000000000",
        "token_uri": "https://oauth2.googleapis.com/token"
    }"#;
    let sa = ServiceAccount::from_json(json).expect("parse");
    assert_eq!(sa.client_email, "test@test-proj.iam.gserviceaccount.com");
    assert_eq!(sa.token_uri, "https://oauth2.googleapis.com/token");
    assert_eq!(sa.private_key_id, "abc123");
    assert!(sa.private_key.contains("BEGIN PRIVATE KEY"));
}

#[test]
fn service_account_rejects_malformed_json() {
    let err = ServiceAccount::from_json("{ not valid json")
        .expect_err("should fail on bad json");
    let msg = err.to_string();
    assert!(msg.contains("service account json"), "got: {msg}");
}

#[test]
fn service_account_rejects_missing_required_field() {
    // Missing client_email
    let json = r#"{
        "private_key": "x",
        "token_uri": "https://oauth2.googleapis.com/token"
    }"#;
    assert!(ServiceAccount::from_json(json).is_err());
}
