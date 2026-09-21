//! Round-trip coverage for `widget: switch` — the tri-state boolean
//! toggle used by the strength equipment fields (paused/belted/
//! wrist_wraps/knee_sleeves). The value contract is a NULLABLE bool:
//! blank cell = not recorded; the form only writes true/false when the
//! user explicitly sets the toggle.

use airledger_engine::parse_input_overlay;
use airledger_engine::schema::input::{InputSpec, WidgetType};

const SWITCH_YAML: &str = r#"
target: strength.view.yml
fields:
  belted:
    widget: switch
    autofill: false
    show_when:
      exercise:
        in: [Barbell Squat, Barbell Deadlift]
  paused:
    widget: switch
"#;

#[test]
fn parses_switch_widget() {
    let overlay = parse_input_overlay(SWITCH_YAML).unwrap();
    let belted = overlay.dimensions["belted"].input.as_ref().unwrap();
    assert_eq!(belted.widget, WidgetType::Switch);
    assert!(!belted.autofill);
    let paused = overlay.dimensions["paused"].input.as_ref().unwrap();
    assert_eq!(paused.widget, WidgetType::Switch);
}

#[test]
fn switch_survives_json_round_trip() {
    // The engine ships InputSpec as JSON to the Dart adapter; a lossy
    // round-trip here would silently drop the widget on device (trap #1).
    let overlay = parse_input_overlay(SWITCH_YAML).unwrap();
    let spec = overlay.dimensions["belted"].input.as_ref().unwrap();
    let json = serde_json::to_string(spec).unwrap();
    assert!(json.contains("\"widget\":\"switch\""), "json: {json}");
    let back: InputSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, spec);
}

#[test]
fn switch_show_when_in_predicate_round_trips() {
    // Conditional visibility for equipment fields is expressed as
    // `show_when: { exercise: { in: [...] } }` — verify the raw mapping
    // survives the overlay parse so eval::is_visible_given sees it.
    let overlay = parse_input_overlay(SWITCH_YAML).unwrap();
    let sw = overlay.dimensions["belted"].show_when.as_ref().unwrap();
    let pred = sw
        .get(serde_yaml::Value::String("exercise".into()))
        .and_then(|v| v.as_mapping())
        .unwrap();
    let list = pred
        .get(serde_yaml::Value::String("in".into()))
        .and_then(|v| v.as_sequence())
        .unwrap();
    assert_eq!(list.len(), 2);
}
