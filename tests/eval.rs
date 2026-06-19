//! Tests for the evaluator layer — show_when predicates, derives,
//! codec round-trips, and template interpolation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use chrono::NaiveDate;

use airledger_engine::{
    apply_derives, apply_overlay, decode, encode, is_visible_given,
    parse_input_overlay, parse_view, CellValue, Record, TemplateInterpolator,
};

fn fitness_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fitness")
}

fn read_str(p: &PathBuf) -> String {
    fs::read_to_string(p).unwrap()
}

// ---------------------------------------------------------------- codec

#[test]
fn codec_string_round_trip() {
    use airledger_engine::schema::view::DimensionType;
    let v = CellValue::String("hello".into());
    let enc = encode(DimensionType::String, &v);
    assert_eq!(enc, CellValue::String("hello".into()));
    let dec = decode(DimensionType::String, "hello");
    assert_eq!(dec, CellValue::String("hello".into()));
}

#[test]
fn codec_number_handles_int_and_float() {
    use airledger_engine::schema::view::DimensionType;
    assert_eq!(decode(DimensionType::Number, "95"), CellValue::Int(95));
    assert_eq!(decode(DimensionType::Number, "1.5"), CellValue::Float(1.5));
    assert_eq!(decode(DimensionType::Number, ""), CellValue::Null);
    // Wire form: numbers stay numeric — Sheets stores them as numbers
    // not text-with-leading-apostrophe.
    let enc = encode(DimensionType::Number, &CellValue::Int(95));
    assert_eq!(enc, CellValue::Int(95));
}

#[test]
fn codec_date_emits_iso() {
    use airledger_engine::schema::view::DimensionType;
    let d = NaiveDate::from_ymd_opt(2026, 6, 19).unwrap();
    let enc = encode(DimensionType::Date, &CellValue::Date(d));
    assert_eq!(enc, CellValue::String("2026-06-19".into()));
    let dec = decode(DimensionType::Date, "2026-06-19");
    assert_eq!(dec, CellValue::Date(d));
}

#[test]
fn codec_bool() {
    use airledger_engine::schema::view::DimensionType;
    assert_eq!(decode(DimensionType::Boolean, "true"), CellValue::Bool(true));
    assert_eq!(decode(DimensionType::Boolean, "FALSE"), CellValue::Bool(false));
    let enc = encode(DimensionType::Boolean, &CellValue::Bool(true));
    assert_eq!(enc, CellValue::Bool(true));
}

// ---------------------------------------------------------------- show_when

#[test]
fn show_when_real_strength_isometric_and_timed() {
    // Loads the real strength schema and walks the show_when on the
    // weight / reps / duration / rpe dims for every group membership.
    let view = parse_view(&read_str(&fitness_root().join("strength.view.yml")))
        .unwrap();
    let overlay =
        parse_input_overlay(&read_str(&fitness_root().join("strength.input.yml")))
            .unwrap();
    let view = apply_overlay(view, overlay).unwrap();
    let groups = &view.groups;

    let mk = |exercise: &str| -> Record {
        let mut r = Record::new();
        r.insert("exercise".into(), CellValue::String(exercise.into()));
        r
    };

    let weight_sw = view.dimension_by_name("weight").unwrap().show_when.as_ref();
    let reps_sw = view.dimension_by_name("reps").unwrap().show_when.as_ref();
    let dur_sw = view.dimension_by_name("duration").unwrap().show_when.as_ref();
    let rpe_sw = view.dimension_by_name("rpe").unwrap().show_when.as_ref();

    // A non-grouped strength exercise → weight + reps + rpe shown,
    // duration hidden.
    let bench = mk("Flat Barbell Bench Press");
    assert!(is_visible_given(weight_sw, &bench, groups));
    assert!(is_visible_given(reps_sw, &bench, groups));
    assert!(!is_visible_given(dur_sw, &bench, groups));
    assert!(is_visible_given(rpe_sw, &bench, groups));

    // Isometric hold (Plank) → only duration shown; weight/reps/rpe
    // hidden via not_in_group: [isometric, timed, mobility] on those
    // fields.
    let plank = mk("Plank");
    assert!(!is_visible_given(weight_sw, &plank, groups));
    assert!(!is_visible_given(reps_sw, &plank, groups));
    assert!(is_visible_given(dur_sw, &plank, groups));
    assert!(!is_visible_given(rpe_sw, &plank, groups));

    // Mobility (Wall Slides) → reps shown, weight + rpe hidden,
    // duration hidden.
    let walls = mk("Wall Slides");
    assert!(!is_visible_given(weight_sw, &walls, groups));
    assert!(is_visible_given(reps_sw, &walls, groups));
    assert!(!is_visible_given(dur_sw, &walls, groups));
    assert!(!is_visible_given(rpe_sw, &walls, groups));

    // Timed (Handstand Practice) → weight/reps/duration/rpe all
    // hidden; only start/end_time + notes carry meaning.
    let handstand = mk("Handstand Practice");
    assert!(!is_visible_given(weight_sw, &handstand, groups));
    assert!(!is_visible_given(reps_sw, &handstand, groups));
    assert!(!is_visible_given(dur_sw, &handstand, groups));
    assert!(!is_visible_given(rpe_sw, &handstand, groups));
}

#[test]
fn show_when_scalar_and_in_predicates() {
    let mut groups = BTreeMap::<String, BTreeSet<String>>::new();
    groups.insert("colors".into(), ["red".into(), "blue".into()].into());

    // Scalar match: `type: treadmill` shows only when type=treadmill.
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(
        "type: treadmill",
    )
    .unwrap();
    let sw = yaml.as_mapping();
    let mut r = Record::new();
    r.insert("type".into(), CellValue::String("treadmill".into()));
    assert!(is_visible_given(sw, &r, &groups));
    r.insert("type".into(), CellValue::String("stairmaster".into()));
    assert!(!is_visible_given(sw, &r, &groups));

    // In-group predicate.
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(
        "kind: { in_group: colors }",
    )
    .unwrap();
    let sw = yaml.as_mapping();
    let mut r = Record::new();
    r.insert("kind".into(), CellValue::String("red".into()));
    assert!(is_visible_given(sw, &r, &groups));
    r.insert("kind".into(), CellValue::String("green".into()));
    assert!(!is_visible_given(sw, &r, &groups));
}

// ---------------------------------------------------------------- derives

#[test]
fn derive_day_of_week_from_date() {
    // Strength's day_of_week is derived from date with format
    // weekday_long.
    let view = parse_view(&read_str(&fitness_root().join("strength.view.yml")))
        .unwrap();
    let overlay =
        parse_input_overlay(&read_str(&fitness_root().join("strength.input.yml")))
            .unwrap();
    let view = apply_overlay(view, overlay).unwrap();

    let mut record = Record::new();
    record.insert(
        "date".into(),
        CellValue::Date(NaiveDate::from_ymd_opt(2026, 6, 19).unwrap()),
    );

    apply_derives(&view, &mut record);
    assert_eq!(
        record.get("day_of_week"),
        Some(&CellValue::String("Friday".into())),
        "2026-06-19 is a Friday",
    );
}

#[test]
fn derive_skips_already_populated() {
    let view = parse_view(&read_str(&fitness_root().join("strength.view.yml")))
        .unwrap();
    let overlay =
        parse_input_overlay(&read_str(&fitness_root().join("strength.input.yml")))
            .unwrap();
    let view = apply_overlay(view, overlay).unwrap();

    let mut record = Record::new();
    record.insert(
        "date".into(),
        CellValue::Date(NaiveDate::from_ymd_opt(2026, 6, 19).unwrap()),
    );
    record.insert(
        "day_of_week".into(),
        CellValue::String("CUSTOM".into()),
    );
    apply_derives(&view, &mut record);
    assert_eq!(
        record.get("day_of_week"),
        Some(&CellValue::String("CUSTOM".into())),
        "existing values aren't overwritten",
    );
}

// ---------------------------------------------------------------- templates

#[test]
fn template_interpolator_round_filter_matches_dart() {
    // The Dart round filter was added because jinja-dart's default
    // round did the wrong thing for `(280 * 0.50 / 5) | round * 5`.
    // The Rust port should produce the SAME number on the same inputs.
    let view = parse_view(&read_str(&fitness_root().join("strength.view.yml")))
        .unwrap();
    let interp = TemplateInterpolator::default();

    let mut vars = Record::new();
    vars.insert("top".into(), CellValue::Int(280));

    let mut entry = Record::new();
    entry.insert(
        "weight".into(),
        CellValue::String("{{ ((top * 0.50 / 5) | round) * 5 }}".into()),
    );
    entry.insert("reps".into(), CellValue::Int(5));

    let rendered = interp.apply(&[entry], &view, &vars).unwrap();
    let r = &rendered[0];

    // 280 * 0.50 = 140, /5 = 28, round = 28, *5 = 140 — coerced back
    // to a number by the codec since `weight` is DimensionType::Number.
    assert_eq!(
        r.get("weight"),
        Some(&CellValue::Int(140)),
        "got: {:?}",
        r.get("weight"),
    );
    assert_eq!(r.get("reps"), Some(&CellValue::Int(5)));
}

#[test]
fn template_interpolator_skips_strings_without_template_syntax() {
    let view = parse_view(&read_str(&fitness_root().join("strength.view.yml")))
        .unwrap();
    let interp = TemplateInterpolator::default();
    let mut vars = Record::new();
    vars.insert("top".into(), CellValue::Int(280));

    let mut entry = Record::new();
    entry.insert(
        "exercise".into(),
        CellValue::String("Barbell Deadlift".into()),
    );
    let rendered = interp.apply(&[entry], &view, &vars).unwrap();
    assert_eq!(
        rendered[0].get("exercise"),
        Some(&CellValue::String("Barbell Deadlift".into())),
        "plain strings should pass through untouched",
    );
}
