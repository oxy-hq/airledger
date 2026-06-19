//! `.input.yml` parser. Mirrors `input_parser.dart`.
//!
//! Accepts two layouts for each per-field block:
//!
//! 1. **Flat** (preferred) — form-spec keys live at the field's top
//!    level alongside `derive` / `show_when`. `options:` is the
//!    canonical place for autocomplete / dropdown values.
//!
//! 2. **Legacy** — form-spec keys nested under `input:`, autocomplete
//!    values under `samples:`.
//!
//! The distinguisher in flat mode is "the field has form-spec keys"
//! (widget / required / default / etc.). A field with only `derive:`
//! produces no `InputSpec`.

use serde_yaml::{Mapping, Value};

use super::ParseError;
use crate::schema::input::{
    InputSpec, ListDisplay, LogFormat, Plannable, PostLog, RepeatGroup,
    TimerLadder, TimerStopFormat, TimerStopTarget, WidgetType,
};
use crate::schema::overlay::{DimensionOverlay, InputOverlay};
use crate::schema::view::{Derive, DeriveFormat};

const FORM_SPEC_KEYS: &[&str] = &[
    "widget",
    "required",
    "default",
    "min",
    "max",
    "options",
    "placeholder",
    "editable",
    "now_button",
    "history",
    "ladders",
    "stop_target",
    "stop_targets",
];

pub fn parse_input_overlay(yaml: &str) -> Result<InputOverlay, ParseError> {
    let v: Value = serde_yaml::from_str(yaml)?;
    let map = v
        .as_mapping()
        .ok_or_else(|| ParseError::Schema("Top-level YAML must be a map".into()))?;

    // `target:` resolves to the paired .view.yml's basename. The view
    // name is the basename minus the `.view.yml` extension.
    let target = map
        .get(Value::String("target".into()))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ParseError::Schema(
                "Missing or malformed `target:` field in .input.yml. \
                 Expected: target: <view_name>.view.yml"
                    .into(),
            )
        })?;
    if !target.ends_with(".view.yml") {
        return Err(ParseError::Schema(format!(
            "`target:` must end with .view.yml (got: {target})"
        )));
    }
    let view_name = target.trim_end_matches(".view.yml").to_string();

    // Per-field overlays. `fields:` is the canonical key; `dimensions:`
    // is accepted for backwards compat with the pre-split schema files.
    let mut dims = std::collections::BTreeMap::new();
    if let Some(node) = map
        .get(Value::String("fields".into()))
        .or_else(|| map.get(Value::String("dimensions".into())))
    {
        let fields = node
            .as_mapping()
            .ok_or_else(|| ParseError::Schema(
                "fields: (or legacy dimensions:) must be a map keyed by \
                 field name".into(),
            ))?;
        for (k, v) in fields {
            let name = k.as_str().ok_or_else(|| {
                ParseError::Schema("field overlay key must be a string".into())
            })?;
            let inner = v.as_mapping().ok_or_else(|| {
                ParseError::Schema(format!(
                    "field overlay for \"{name}\" must be a map"
                ))
            })?;
            dims.insert(name.to_string(), parse_dimension_overlay(inner)?);
        }
    }

    let groups = parse_groups(map.get(Value::String("groups".into())));

    Ok(InputOverlay {
        view_name,
        date_field: map.get(Value::String("date_field".into()))
            .and_then(Value::as_str)
            .map(String::from),
        plannable: map.get(Value::String("plannable".into()))
            .and_then(Value::as_mapping)
            .map(parse_plannable)
            .transpose()?,
        list_display: map.get(Value::String("list_display".into()))
            .and_then(Value::as_mapping)
            .map(parse_list_display)
            .transpose()?,
        spreadsheet_id: map.get(Value::String("spreadsheet_id".into()))
            .and_then(Value::as_str)
            .map(String::from),
        icon: map.get(Value::String("icon".into()))
            .and_then(Value::as_str)
            .map(String::from),
        post_log: map.get(Value::String("post_log".into()))
            .and_then(Value::as_mapping)
            .map(parse_post_log)
            .transpose()?,
        dimensions: dims,
        groups,
        top_metric: map.get(Value::String("top_metric".into()))
            .and_then(Value::as_str)
            .map(String::from),
        repeat_group: map.get(Value::String("repeat_group".into()))
            .and_then(Value::as_mapping)
            .map(parse_repeat_group)
            .transpose()?,
    })
}

fn parse_dimension_overlay(node: &Mapping) -> Result<DimensionOverlay, ParseError> {
    let legacy_input = node
        .get(Value::String("input".into()))
        .and_then(Value::as_mapping);
    let is_legacy = legacy_input.is_some();

    let form_source: &Mapping = legacy_input.unwrap_or(node);
    let has_input_config = is_legacy || looks_like_form_spec(form_source);

    // Allowed/suggested values: `options:` (preferred) → `samples:`
    // (legacy autocomplete location) → `input.options:` (legacy
    // dropdown).
    let options_node = node
        .get(Value::String("options".into()))
        .or_else(|| node.get(Value::String("samples".into())))
        .or_else(|| {
            legacy_input.and_then(|m| m.get(Value::String("options".into())))
        });

    let samples = options_node.and_then(|v| {
        v.as_sequence().map(|s| {
            s.iter()
                .map(|x| match x {
                    Value::String(s) => s.clone(),
                    other => yaml_scalar_to_string(other),
                })
                .collect()
        })
    });

    let input = if has_input_config {
        Some(parse_input_spec(form_source)?)
    } else {
        None
    };

    let derive = node
        .get(Value::String("derive".into()))
        .and_then(Value::as_mapping)
        .map(parse_derive)
        .transpose()?;

    let show_when = node
        .get(Value::String("show_when".into()))
        .and_then(Value::as_mapping)
        .cloned();

    Ok(DimensionOverlay { input, samples, show_when, derive })
}

fn looks_like_form_spec(node: &Mapping) -> bool {
    FORM_SPEC_KEYS
        .iter()
        .any(|k| node.contains_key(Value::String((*k).into())))
}

fn parse_input_spec(node: &Mapping) -> Result<InputSpec, ParseError> {
    let widget_str = node
        .get(Value::String("widget".into()))
        .and_then(Value::as_str)
        .unwrap_or("text");
    let widget = parse_widget_type(widget_str)?;

    let options = node
        .get(Value::String("options".into()))
        .and_then(Value::as_sequence)
        .map(|s| {
            s.iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => yaml_scalar_to_string(other),
                })
                .collect()
        });

    let ladders = node
        .get(Value::String("ladders".into()))
        .and_then(Value::as_sequence)
        .map(|s| parse_ladders(s.as_slice()))
        .transpose()?;

    let stop_targets = parse_stop_targets(node)?;

    Ok(InputSpec {
        widget,
        required: node
            .get(Value::String("required".into()))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        default_value: node.get(Value::String("default".into())).cloned(),
        min: node.get(Value::String("min".into())).and_then(Value::as_f64),
        max: node.get(Value::String("max".into())).and_then(Value::as_f64),
        options,
        placeholder: node
            .get(Value::String("placeholder".into()))
            .and_then(Value::as_str)
            .map(String::from),
        editable: node
            .get(Value::String("editable".into()))
            .and_then(Value::as_bool)
            .unwrap_or(true),
        now_button: node
            .get(Value::String("now_button".into()))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        history: node
            .get(Value::String("history".into()))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ladders,
        stop_targets,
    })
}

fn parse_widget_type(s: &str) -> Result<WidgetType, ParseError> {
    Ok(match s {
        "text" => WidgetType::Text,
        "longtext" => WidgetType::Longtext,
        "number" => WidgetType::Number,
        "date" => WidgetType::Date,
        "datetime" => WidgetType::Datetime,
        "timer" => WidgetType::Timer,
        "dropdown" => WidgetType::Dropdown,
        "autocomplete" => WidgetType::Autocomplete,
        other => {
            return Err(ParseError::Schema(format!(
                "Unknown widget type: {other}"
            )))
        }
    })
}

fn parse_ladders(seq: &[Value]) -> Result<Vec<TimerLadder>, ParseError> {
    seq.iter()
        .map(|v| {
            let m = v.as_mapping().ok_or_else(|| {
                ParseError::Schema(
                    "ladders[]: each entry must be a map with label + target"
                        .into(),
                )
            })?;
            let label = require_string(m, "label")?;
            let target = require_string(m, "target")?;
            Ok(TimerLadder { label, target })
        })
        .collect()
}

fn parse_stop_targets(
    node: &Mapping,
) -> Result<Option<Vec<TimerStopTarget>>, ParseError> {
    // Preferred shape: stop_targets: [{target, format}, ...]
    if let Some(list) = node
        .get(Value::String("stop_targets".into()))
        .and_then(Value::as_sequence)
    {
        let parsed = list
            .iter()
            .map(|v| {
                let m = v.as_mapping().ok_or_else(|| {
                    ParseError::Schema(
                        "stop_targets[]: each entry must be a map with \
                         target [+ format]".into(),
                    )
                })?;
                let target = require_string(m, "target")?;
                let format = m
                    .get(Value::String("format".into()))
                    .and_then(Value::as_str)
                    .map(parse_stop_format)
                    .transpose()?
                    .unwrap_or(TimerStopFormat::Elapsed);
                Ok(TimerStopTarget { target, format })
            })
            .collect::<Result<Vec<_>, ParseError>>()?;
        return Ok(Some(parsed));
    }

    // Legacy shortcut: stop_target: <name>
    if let Some(single) = node
        .get(Value::String("stop_target".into()))
        .and_then(Value::as_str)
    {
        return Ok(Some(vec![TimerStopTarget {
            target: single.to_string(),
            format: TimerStopFormat::Elapsed,
        }]));
    }

    Ok(None)
}

fn parse_stop_format(s: &str) -> Result<TimerStopFormat, ParseError> {
    Ok(match s {
        "elapsed" => TimerStopFormat::Elapsed,
        "seconds" => TimerStopFormat::Seconds,
        "time_of_day" => TimerStopFormat::TimeOfDay,
        other => {
            return Err(ParseError::Schema(format!(
                "Unknown timer stop format: {other}"
            )))
        }
    })
}

fn parse_derive(node: &Mapping) -> Result<Derive, ParseError> {
    let from = require_string(node, "from")?;
    let format_str = require_string(node, "format")?;
    let format = match format_str.as_str() {
        "weekday_long" => DeriveFormat::WeekdayLong,
        "weekday_short" => DeriveFormat::WeekdayShort,
        "iso_date" => DeriveFormat::IsoDate,
        "iso_date_time" => DeriveFormat::IsoDateTime,
        other => {
            return Err(ParseError::Schema(format!(
                "Unknown derive format: {other}"
            )))
        }
    };
    Ok(Derive { from, format })
}

fn parse_list_display(node: &Mapping) -> Result<ListDisplay, ParseError> {
    Ok(ListDisplay {
        title: require_string(node, "title")?,
        subtitle: node
            .get(Value::String("subtitle".into()))
            .and_then(Value::as_str)
            .map(String::from),
    })
}

fn parse_plannable(node: &Mapping) -> Result<Plannable, ParseError> {
    let log_format = match require_string(node, "log_format")?.as_str() {
        "time_string" => LogFormat::TimeString,
        "iso_time" => LogFormat::IsoTime,
        "iso_date_time" => LogFormat::IsoDateTime,
        other => {
            return Err(ParseError::Schema(format!(
                "Unknown log format: {other}"
            )))
        }
    };
    Ok(Plannable {
        log_field: require_string(node, "log_field")?,
        log_format,
    })
}

fn parse_post_log(node: &Mapping) -> Result<PostLog, ParseError> {
    Ok(PostLog {
        model: require_string(node, "model")?,
        prompt: require_string(node, "prompt")?,
    })
}

fn parse_repeat_group(node: &Mapping) -> Result<RepeatGroup, ParseError> {
    let fields = node
        .get(Value::String("fields".into()))
        .and_then(Value::as_sequence)
        .ok_or_else(|| {
            ParseError::Schema(
                "repeat_group.fields: must be a non-empty list of field names"
                    .into(),
            )
        })?;
    if fields.is_empty() {
        return Err(ParseError::Schema(
            "repeat_group.fields: must be a non-empty list".into(),
        ));
    }
    let fields: Vec<String> = fields
        .iter()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => yaml_scalar_to_string(other),
        })
        .collect();
    let label = require_string(node, "label")?;
    let min = node
        .get(Value::String("min".into()))
        .and_then(Value::as_u64)
        .unwrap_or(1) as usize;
    let group_key = node
        .get(Value::String("group_key".into()))
        .and_then(Value::as_str)
        .map(String::from);
    Ok(RepeatGroup { fields, label, min, group_key })
}

fn parse_groups(
    node: Option<&Value>,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let Some(m) = node.and_then(Value::as_mapping) else {
        return Default::default();
    };
    let mut out = std::collections::BTreeMap::new();
    for (k, v) in m {
        let Some(name) = k.as_str() else { continue };
        let Some(values) = v.as_sequence() else { continue };
        let set: std::collections::BTreeSet<String> = values
            .iter()
            .map(|x| match x {
                Value::String(s) => s.clone(),
                other => yaml_scalar_to_string(other),
            })
            .collect();
        out.insert(name.to_string(), set);
    }
    out
}

fn require_string(node: &Mapping, key: &str) -> Result<String, ParseError> {
    node.get(Value::String(key.into()))
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| {
            ParseError::Schema(format!("Missing or non-string field: {key}"))
        })
}

/// Convert a non-string YAML scalar to its display string. Used for
/// lists like `options:` and `samples:` that the Dart side coerces to
/// strings (e.g. when an option is a bare number).
fn yaml_scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        // For nested maps/seqs we just emit a debug-like rendering.
        // The Dart side wouldn't allow this either, so callers are
        // expected to validate upstream.
        other => format!("{other:?}"),
    }
}
