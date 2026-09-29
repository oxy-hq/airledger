//! Round-trip coverage for `widget: video` — the video-attach
//! affordance used by the strength form (`video_url`). The dim's value
//! is a Google Photos deep link written by the app's Photos Picker
//! flow; the widget never accepts free text. `autofill: false` is the
//! norm (a carried-over video would attach the WRONG set's footage —
//! fabricated data).

use airledger_engine::parse_input_overlay;
use airledger_engine::schema::input::{InputSpec, WidgetType};

const VIDEO_YAML: &str = r#"
target: strength.view.yml
fields:
  video_url:
    widget: video
    autofill: false
  video_media_id:
    editable: false
"#;

#[test]
fn parses_video_widget() {
    let overlay = parse_input_overlay(VIDEO_YAML).unwrap();
    let video = overlay.dimensions["video_url"].input.as_ref().unwrap();
    assert_eq!(video.widget, WidgetType::Video);
    assert!(!video.autofill);
    // The paired media-id column is app-written only.
    let media = overlay.dimensions["video_media_id"].input.as_ref().unwrap();
    assert!(!media.editable);
}

#[test]
fn video_survives_json_round_trip() {
    // The engine ships InputSpec as JSON to the Dart adapter; a lossy
    // round-trip here would silently drop the widget on device (trap #1).
    let overlay = parse_input_overlay(VIDEO_YAML).unwrap();
    let spec = overlay.dimensions["video_url"].input.as_ref().unwrap();
    let json = serde_json::to_string(spec).unwrap();
    assert!(json.contains("\"widget\":\"video\""), "json: {json}");
    let back: InputSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, spec);
}

#[test]
fn unknown_widget_still_errors_loudly() {
    // Guard against the arm accidentally widening to a catch-all.
    let bad = "target: strength.view.yml\nfields:\n  x:\n    widget: hologram\n";
    let err = parse_input_overlay(bad).unwrap_err();
    assert!(format!("{err}").contains("Unknown widget type"));
}
