//! Round-trip coverage for the HR-driven timer schema keys
//! (`ladders[].hr_pct`, `hr_max_target`) — Whoop live HR integration.

use airledger_engine::parse_input_overlay;
use airledger_engine::schema::input::InputSpec;

const HR_YAML: &str = r#"
target: cardio.view.yml
fields:
  start_time:
    widget: timer
    hr_max_target: max_hr
    ladders:
      - { label: "Zone 4 reached", target: zone4_reached, hr_pct: 80 }
      - { label: "Zone 5 reached", target: zone5_reached }
    stop_target: total_time
"#;

#[test]
fn parses_hr_pct_and_hr_max_target() {
    let overlay = parse_input_overlay(HR_YAML).unwrap();
    let spec = overlay.dimensions["start_time"].input.as_ref().unwrap();
    assert_eq!(spec.hr_max_target.as_deref(), Some("max_hr"));
    let ladders = spec.ladders.as_ref().unwrap();
    assert_eq!(ladders[0].hr_pct, Some(80.0));
    assert_eq!(ladders[1].hr_pct, None);
}

#[test]
fn hr_keys_survive_json_round_trip() {
    // The Dart adapter ships InputSpec as JSON into the engine; a lossy
    // round-trip here would silently drop the feature on device.
    let overlay = parse_input_overlay(HR_YAML).unwrap();
    let spec = overlay.dimensions["start_time"].input.as_ref().unwrap();
    let json = serde_json::to_string(spec).unwrap();
    let back: InputSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, spec);
}

#[test]
fn absent_hr_keys_stay_none_and_are_not_serialized() {
    let overlay = parse_input_overlay(
        "target: cardio.view.yml\nfields:\n  start_time:\n    widget: timer\n    ladders:\n      - { label: Z4, target: zone4_reached }\n",
    )
    .unwrap();
    let spec = overlay.dimensions["start_time"].input.as_ref().unwrap();
    assert_eq!(spec.hr_max_target, None);
    assert_eq!(spec.ladders.as_ref().unwrap()[0].hr_pct, None);
    let json = serde_json::to_string(spec).unwrap();
    assert!(!json.contains("hr_pct"), "json: {json}");
    assert!(!json.contains("hr_max_target"), "json: {json}");
}
