//! `.view.yml` parser. Mirrors `schema_parser.dart` — semantic layer
//! only; `input`, `samples`, `derive`, `show_when` blocks nested under
//! dimensions are intentionally ignored here (overlay-only).

use serde::Deserialize;
use serde_yaml::Mapping;

use super::ParseError;
use crate::schema::view::{
    Dimension, DimensionType, Entity, EntityKind, Measure, MeasureType,
    ViewSchema,
};

/// Parse a `.view.yml` document. Drops any input-layer-only fields
/// that happen to be nested on dimensions in the source file (legacy
/// shape) so the returned [`ViewSchema`] is pure semantic layer.
pub fn parse_view(yaml: &str) -> Result<ViewSchema, ParseError> {
    let raw: RawView = serde_yaml::from_str(yaml)?;
    let dimensions = raw
        .dimensions
        .into_iter()
        .map(|d| Dimension {
            name: d.name,
            kind: d.kind,
            expr: d.expr.unwrap_or_else(|| {
                // Default expr = name (matches the Dart parser).
                String::new()
            }),
            description: d.description,
            // Overlay-only fields stay None at the view-parse stage.
            samples: None,
            input: None,
            derive: None,
            show_when: None,
        })
        // Patch up the expr default — needs the dim's name in scope.
        .scan((), |_, mut dim| {
            if dim.expr.is_empty() {
                dim.expr = dim.name.clone();
            }
            Some(dim)
        })
        .collect();

    let measures = raw
        .measures
        .unwrap_or_default()
        .into_iter()
        .map(|m| Measure {
            name: m.name,
            kind: m.kind,
            expr: m.expr,
            description: m.description,
        })
        .collect();

    let entities = raw
        .entities
        .unwrap_or_default()
        .into_iter()
        .map(|e| Entity {
            name: e.name,
            kind: e.kind,
            keys: e.keys.unwrap_or_default(),
        })
        .collect();

    Ok(ViewSchema {
        name: raw.name,
        description: raw.description,
        datasource: raw.datasource,
        table: raw.table,
        date_field: None,
        spreadsheet_id: None,
        entities,
        dimensions,
        measures,
        list_display: None,
        plannable: None,
        icon: None,
        post_log: None,
        groups: Default::default(),
        top_metric: None,
        has_input_overlay: false,
        repeat_group: None,
    })
}

// --- Raw deserialization shape ----------------------------------------------
//
// These mirror the YAML structure 1:1 so serde can do the heavy lift.
// They get massaged into the public types above. Mostly differences are:
// - dimensions may carry input-layer noise (`input:`, `samples:`, ...)
//   in legacy YAML; we accept-and-ignore it.
// - top-level legacy keys like `date_field`, `plannable`, etc. could
//   appear on `.view.yml` files written before the input-layer split;
//   we accept and discard them at the view-parse stage (they belong in
//   `.input.yml`).

#[derive(Debug, Deserialize)]
struct RawView {
    name: String,
    #[serde(default)]
    description: Option<String>,
    datasource: String,
    table: String,
    #[serde(default)]
    entities: Option<Vec<RawEntity>>,
    dimensions: Vec<RawDimension>,
    #[serde(default)]
    measures: Option<Vec<RawMeasure>>,
    // Legacy / mistake-tolerant: ignore input-layer keys at the top
    // level. They belong in the .input.yml overlay.
    #[serde(default)]
    #[allow(dead_code)]
    spreadsheet_id: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    date_field: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawEntity {
    name: String,
    #[serde(rename = "type")]
    kind: EntityKind,
    #[serde(default)]
    keys: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct RawDimension {
    name: String,
    #[serde(rename = "type")]
    kind: DimensionType,
    #[serde(default)]
    expr: Option<String>,
    #[serde(default)]
    description: Option<String>,
    // Accept-and-ignore: input-layer fields can be nested here in
    // legacy single-file shape. We strip them out at this stage so
    // schema_parser stays pure semantic.
    #[serde(default)]
    #[allow(dead_code)]
    input: Option<Mapping>,
    #[serde(default)]
    #[allow(dead_code)]
    samples: Option<Vec<String>>,
    #[serde(default)]
    #[allow(dead_code)]
    derive: Option<Mapping>,
    #[serde(default)]
    #[allow(dead_code)]
    show_when: Option<Mapping>,
}

#[derive(Debug, Deserialize)]
struct RawMeasure {
    name: String,
    #[serde(rename = "type")]
    kind: MeasureType,
    #[serde(default)]
    expr: Option<String>,
    #[serde(default)]
    description: Option<String>,
}
