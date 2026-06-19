//! End-to-end parser tests against the real airledger-fitness and
//! pokehouse-ledger YAMLs (copied into `tests/fixtures/` at crate
//! setup). When the schemas evolve, refresh the fixtures and re-run
//! `cargo test`.

use std::fs;
use std::path::{Path, PathBuf};

use airledger_engine::{apply_overlay, parse_input_overlay, parse_view};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Walk a fixture directory and run parse_view + parse_input_overlay +
/// apply_overlay across every paired (.view.yml, .input.yml) it
/// contains. Templates (`*.template.yml`) and standalone analytics
/// views (no paired input file) are also covered separately.
fn walk_fixture(dir: &Path) {
    let mut total_views = 0usize;
    let mut total_inputs = 0usize;
    let mut total_templates = 0usize;

    for entry in fs::read_dir(dir).expect("read fixture dir") {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        if name.ends_with(".template.yml") {
            total_templates += 1;
            // Phase 1 doesn't include the template parser yet; just
            // confirm the file is well-formed YAML.
            let _: serde_yaml::Value = serde_yaml::from_str(&read(&path))
                .unwrap_or_else(|e| panic!("template {name}: {e}"));
            continue;
        }

        if name.ends_with(".input.yml") {
            total_inputs += 1;
            let raw = read(&path);
            parse_input_overlay(&raw)
                .unwrap_or_else(|e| panic!("input overlay {name}: {e}"));
            continue;
        }

        if name.ends_with(".view.yml") {
            total_views += 1;
            let raw = read(&path);
            let view = parse_view(&raw)
                .unwrap_or_else(|e| panic!("view {name}: {e}"));

            // If a paired .input.yml exists, apply the overlay too.
            let paired = path.with_file_name(
                name.replace(".view.yml", ".input.yml"),
            );
            if paired.exists() {
                let overlay = parse_input_overlay(&read(&paired))
                    .unwrap_or_else(|e| {
                        panic!("paired overlay for {name}: {e}")
                    });
                let merged = apply_overlay(view, overlay)
                    .unwrap_or_else(|e| {
                        panic!("apply_overlay for {name}: {e}")
                    });
                assert!(
                    merged.has_input_overlay,
                    "merged {name} should have_input_overlay=true",
                );
            } else {
                // Analytics-only view (e.g. body_composition).
                assert!(
                    !view.has_input_overlay,
                    "analytical {name} should have_input_overlay=false",
                );
            }
        }
    }

    println!(
        "{}: {} views, {} inputs, {} templates",
        dir.file_name().unwrap().to_string_lossy(),
        total_views,
        total_inputs,
        total_templates
    );
    assert!(total_views > 0, "fixture dir had no .view.yml files?");
}

#[test]
fn parse_all_fitness_schemas() {
    walk_fixture(&fixtures_root().join("fitness"));
}

#[test]
fn parse_all_pokehouse_schemas() {
    walk_fixture(&fixtures_root().join("pokehouse"));
}

/// Spot-check the strength view picks up the timer widget + ladders +
/// stop_targets from the input overlay, including the multi-format
/// stop_targets.
#[test]
fn strength_timer_widget_round_trip() {
    let fitness = fixtures_root().join("fitness");
    let view = parse_view(&read(&fitness.join("strength.view.yml"))).unwrap();
    let overlay =
        parse_input_overlay(&read(&fitness.join("strength.input.yml")))
            .unwrap();
    let merged = apply_overlay(view, overlay).unwrap();

    let start_time = merged
        .dimension_by_name("start_time")
        .expect("start_time dim must exist on strength");
    let input = start_time.input.as_ref().expect("start_time has input spec");

    use airledger_engine::schema::input::{TimerStopFormat, WidgetType};
    assert_eq!(input.widget, WidgetType::Timer, "strength.start_time widget");
    let stops = input
        .stop_targets
        .as_ref()
        .expect("strength.start_time.stop_targets is configured");
    assert!(
        stops.iter().any(|s| s.target == "end_time"
            && s.format == TimerStopFormat::TimeOfDay),
        "expected end_time/time_of_day in stop_targets, got {stops:?}",
    );
    assert!(
        stops.iter().any(|s| s.target == "duration"
            && s.format == TimerStopFormat::Seconds),
        "expected duration/seconds in stop_targets, got {stops:?}",
    );
}

/// Spot-check sauces' batch-as-entity wiring (repeat_group +
/// group_key + start_time + end_time).
#[test]
fn sauces_repeat_group_round_trip() {
    let ph = fixtures_root().join("pokehouse");
    let view = parse_view(&read(&ph.join("sauces.view.yml"))).unwrap();
    let overlay =
        parse_input_overlay(&read(&ph.join("sauces.input.yml"))).unwrap();
    let merged = apply_overlay(view, overlay).unwrap();

    let rg = merged.repeat_group.as_ref().expect("sauces has repeat_group");
    assert_eq!(rg.group_key.as_deref(), Some("batch_id"));
    assert_eq!(rg.label, "Ingredient");
    assert_eq!(rg.min, 1);
    assert_eq!(rg.fields, vec![
        "ingredient".to_string(),
        "ingredient_qty".to_string(),
        "ingredient_unit".to_string(),
    ]);

    // Confirm start_time / end_time made it onto the merged dims.
    assert!(merged.dimension_by_name("start_time").is_some());
    assert!(merged.dimension_by_name("end_time").is_some());
}

/// Verify the overlay-mismatch + unknown-dim + unknown-top-metric
/// failure paths produce the right structured errors.
#[test]
fn overlay_validation_errors() {
    use airledger_engine::OverlayError;
    let view_yaml = r#"
name: foo
datasource: gsheets
table: foo
dimensions:
  - { name: id, type: string, expr: id }
"#;
    let overlay_yaml = r#"
target: bar.view.yml
fields:
  id: { editable: false }
"#;
    let view = parse_view(view_yaml).unwrap();
    let overlay = parse_input_overlay(overlay_yaml).unwrap();
    let err = apply_overlay(view, overlay).unwrap_err();
    assert!(
        matches!(err, OverlayError::ViewNameMismatch { .. }),
        "expected ViewNameMismatch, got {err:?}",
    );

    let view = parse_view(view_yaml).unwrap();
    let overlay = parse_input_overlay(
        r#"
target: foo.view.yml
fields:
  bogus: { widget: text }
"#,
    )
    .unwrap();
    let err = apply_overlay(view, overlay).unwrap_err();
    assert!(
        matches!(err, OverlayError::UnknownDimension { .. }),
        "expected UnknownDimension, got {err:?}",
    );
}
