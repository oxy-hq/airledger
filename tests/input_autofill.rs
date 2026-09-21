//! Round-trip coverage for the per-field `autofill` input-overlay key.
//! `autofill: false` opts a field out of exercise-history autofill in
//! the app's form (subjective per-set fields like rpe/notes must never
//! carry over from a previous session).

use airledger_engine::parse_input_overlay;
use airledger_engine::schema::input::InputSpec;

const AUTOFILL_YAML: &str = r#"
target: strength.view.yml
fields:
  rpe:
    widget: number
    autofill: false
  weight:
    widget: number
    required: true
"#;

#[test]
fn parses_autofill_false_and_defaults_to_true() {
    let overlay = parse_input_overlay(AUTOFILL_YAML).unwrap();
    let rpe = overlay.dimensions["rpe"].input.as_ref().unwrap();
    assert!(!rpe.autofill);
    // Absent → true (autofill is opt-out).
    let weight = overlay.dimensions["weight"].input.as_ref().unwrap();
    assert!(weight.autofill);
}

#[test]
fn autofill_survives_json_round_trip() {
    // The engine ships InputSpec as JSON to the Dart adapter; a lossy
    // round-trip here would silently drop the key on device.
    let overlay = parse_input_overlay(AUTOFILL_YAML).unwrap();
    let spec = overlay.dimensions["rpe"].input.as_ref().unwrap();
    let json = serde_json::to_string(spec).unwrap();
    assert!(json.contains("\"autofill\":false"), "json: {json}");
    let back: InputSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, spec);
}

#[test]
fn autofill_absent_in_json_deserializes_true() {
    // Older serialized views won't carry the key at all.
    let spec: InputSpec =
        serde_json::from_str(r#"{"widget":"number"}"#).unwrap();
    assert!(spec.autofill);
}

#[test]
fn autofill_alone_marks_field_as_form_spec() {
    // A field whose only form key is `autofill:` must still produce an
    // InputSpec (FORM_SPEC_KEYS membership).
    let overlay = parse_input_overlay(
        "target: strength.view.yml\nfields:\n  notes:\n    autofill: false\n",
    )
    .unwrap();
    let notes = overlay.dimensions["notes"].input.as_ref().unwrap();
    assert!(!notes.autofill);
}
