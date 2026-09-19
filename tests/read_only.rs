//! Round-trip tests for the `read_only` input-overlay key.
//!
//! Mirrors the `input_hr.rs` pattern: parse YAML → apply overlay →
//! assert struct fields → JSON round-trip.

use airledger_engine::{apply_overlay, parse_input_overlay, parse_view, ViewSchema};

/// Minimal view YAML reused across tests.
const VIEW_YAML: &str = r#"
name: daily_notes
datasource: gsheets
table: daily_notes
dimensions:
  - { name: date, type: date, expr: date }
  - { name: note, type: string, expr: note }
"#;

/// Input overlay that explicitly sets `read_only: true`.
const INPUT_READ_ONLY_TRUE: &str = r#"
target: daily_notes.view.yml
date_field: date
read_only: true
"#;

/// Input overlay without the `read_only` key (absent = false).
const INPUT_NO_READ_ONLY: &str = r#"
target: daily_notes.view.yml
date_field: date
"#;

/// Input overlay with `read_only: false` (explicit false).
const INPUT_READ_ONLY_FALSE: &str = r#"
target: daily_notes.view.yml
date_field: date
read_only: false
"#;

// ── overlay parsing ────────────────────────────────────────────────────────

#[test]
fn overlay_read_only_true_parses() {
    let overlay = parse_input_overlay(INPUT_READ_ONLY_TRUE).unwrap();
    assert!(
        overlay.read_only,
        "parsed overlay should carry read_only = true"
    );
}

#[test]
fn overlay_read_only_absent_defaults_false() {
    let overlay = parse_input_overlay(INPUT_NO_READ_ONLY).unwrap();
    assert!(
        !overlay.read_only,
        "absent read_only key should default to false"
    );
}

#[test]
fn overlay_read_only_explicit_false() {
    let overlay = parse_input_overlay(INPUT_READ_ONLY_FALSE).unwrap();
    assert!(
        !overlay.read_only,
        "explicit read_only: false should be false"
    );
}

// ── apply_overlay propagation ──────────────────────────────────────────────

#[test]
fn apply_overlay_propagates_read_only_true() {
    let view = parse_view(VIEW_YAML).unwrap();
    let overlay = parse_input_overlay(INPUT_READ_ONLY_TRUE).unwrap();
    let merged = apply_overlay(view, overlay).unwrap();
    assert!(
        merged.read_only,
        "merged ViewSchema should have read_only = true"
    );
}

#[test]
fn apply_overlay_propagates_read_only_false_when_absent() {
    let view = parse_view(VIEW_YAML).unwrap();
    let overlay = parse_input_overlay(INPUT_NO_READ_ONLY).unwrap();
    let merged = apply_overlay(view, overlay).unwrap();
    assert!(
        !merged.read_only,
        "merged ViewSchema should have read_only = false when absent"
    );
}

// ── JSON serialization of ViewSchema ──────────────────────────────────────

#[test]
fn view_schema_json_includes_read_only_true() {
    let view = parse_view(VIEW_YAML).unwrap();
    let overlay = parse_input_overlay(INPUT_READ_ONLY_TRUE).unwrap();
    let merged = apply_overlay(view, overlay).unwrap();
    let json = serde_json::to_string(&merged).unwrap();
    assert!(
        json.contains(r#""read_only":true"#),
        r#"JSON must contain "read_only":true — got: {json}"#
    );
}

#[test]
fn view_schema_json_includes_read_only_false_when_absent() {
    let view = parse_view(VIEW_YAML).unwrap();
    let overlay = parse_input_overlay(INPUT_NO_READ_ONLY).unwrap();
    let merged = apply_overlay(view, overlay).unwrap();
    let json = serde_json::to_string(&merged).unwrap();
    assert!(
        json.contains(r#""read_only":false"#),
        r#"JSON must contain "read_only":false — got: {json}"#
    );
}

// ── JSON deserialization back-compat ───────────────────────────────────────

#[test]
fn view_schema_json_missing_read_only_deserializes_as_false() {
    // Simulates old JSON produced by an engine that didn't know about read_only.
    let old_json = r#"{
        "name": "daily_notes",
        "datasource": "gsheets",
        "table": "daily_notes",
        "dimensions": [
            { "name": "date", "type": "date", "expr": "date" },
            { "name": "note", "type": "string", "expr": "note" }
        ]
    }"#;
    let view: ViewSchema = serde_json::from_str(old_json).unwrap();
    assert!(
        !view.read_only,
        "old JSON without read_only key must deserialize as false"
    );
}

// ── full JSON round-trip ───────────────────────────────────────────────────

#[test]
fn view_schema_json_round_trip_preserves_read_only() {
    let view = parse_view(VIEW_YAML).unwrap();
    let overlay = parse_input_overlay(INPUT_READ_ONLY_TRUE).unwrap();
    let merged = apply_overlay(view, overlay).unwrap();
    let json = serde_json::to_string(&merged).unwrap();
    let back: ViewSchema = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.read_only, merged.read_only,
        "JSON round-trip must preserve read_only"
    );
}
