//! The input/UI layer — widgets, defaults, derive, show_when, groups,
//! plannable, list_display, repeat_group, top_metric. Owned by
//! airledger; never seen by analytical consumers like airlayer/oxy.

use serde::{Deserialize, Serialize};

/// Per-dimension entry-form config. Comes from the `.input.yml`'s
/// `fields.<name>:` block.
///
/// Two YAML shapes are accepted by the parser and normalized to this
/// single struct:
///
/// 1. **Flat** (preferred): widget/required/default/etc. live directly
///    under the field name.
///    ```yaml
///    weight: { widget: number, required: true, min: 0 }
///    ```
///
/// 2. **Legacy nested**: form keys nested under an `input:` block,
///    with autocomplete values under `samples:`.
///    ```yaml
///    weight:
///      input: { widget: number, required: true }
///      samples: [a, b]
///    ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputSpec {
    pub widget: WidgetType,

    #[serde(default)]
    pub required: bool,

    /// Resolved at form-create time. Recognized special strings:
    /// `"now"` (current datetime), `"today"` (current date). Anything
    /// else is used verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<serde_yaml::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,

    /// Allowed values for dropdown / autocomplete-restricted fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,

    /// `false` hides the field from the form (read-only / system-set).
    /// Defaults to `true`.
    #[serde(default = "ret_true")]
    pub editable: bool,

    /// Renders a clock-icon suffix that stamps the current time
    /// (formatted `h:mm:ss a`) when tapped. Used for start_time-style
    /// fields.
    #[serde(default)]
    pub now_button: bool,

    /// Renders a history-icon suffix that opens the per-value history
    /// modal. Opt-in per field.
    #[serde(default)]
    pub history: bool,

    /// For `widget: timer` only. Each entry adds a "tap when reached"
    /// chip that writes elapsed `m:ss` / `H:MM:SS` into the named
    /// target field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladders: Option<Vec<TimerLadder>>,

    /// For `widget: timer` only. Each entry is a target the Stop
    /// button writes into, with a [`TimerStopFormat`] controlling the
    /// encoding (elapsed string, raw seconds as number, or
    /// time-of-day). Multiple entries let one timer fan out; show_when
    /// drops whichever doesn't apply at save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_targets: Option<Vec<TimerStopTarget>>,
}

fn ret_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetType {
    Text,
    Longtext,
    Number,
    Date,
    Datetime,
    Dropdown,
    Autocomplete,
    /// Stopwatch input. Owns a state machine
    /// (idle → running → paused → stopped). Configured via [`ladders`]
    /// and [`stop_targets`] on the parent [`InputSpec`].
    Timer,
}

/// One chip in a [`WidgetType::Timer`]'s ladder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerLadder {
    pub label: String,
    /// Dim name on the same view that elapsed time gets stamped into.
    pub target: String,
}

/// One target the Stop button writes into.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerStopTarget {
    pub target: String,
    #[serde(default = "default_stop_format")]
    pub format: TimerStopFormat,
}

fn default_stop_format() -> TimerStopFormat {
    TimerStopFormat::Elapsed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerStopFormat {
    /// `m:ss` / `H:MM:SS` string — the same shape ladder chips emit.
    Elapsed,
    /// Total elapsed as integer seconds — for numeric fields like
    /// `duration`.
    Seconds,
    /// Current wall-clock as `h:mm:ss a` string — for fields like
    /// `end_time` where the moment of stop matters, not duration.
    TimeOfDay,
}

/// Title/subtitle template for the row tile in the timeline. The
/// subtitle template uses `${field}` placeholders (Sheets-style).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListDisplay {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
}

/// "Plan then log" workflow config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plannable {
    pub log_field: String,
    pub log_format: LogFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    TimeString,
    IsoTime,
    IsoDateTime,
}

/// Post-log LLM hook config. The prompt is rendered as Jinja with the
/// just-logged row + view context + history callables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostLog {
    pub model: String,
    pub prompt: String,
}

/// Declares that a subset of fields repeats together within a single
/// form session. The form renders shared fields once, then N blocks
/// of the repeat fields with a `+ Add <label>` button. On save the
/// form fans out into N records that share every shared field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepeatGroup {
    pub fields: Vec<String>,
    pub label: String,
    #[serde(default = "default_repeat_min")]
    pub min: usize,
    /// Optional dim name that holds the batch UUID — the value shared
    /// across all rows in one save. When set, the form auto-fills it;
    /// the timeline groups rows by it; edit/delete operate on the
    /// whole batch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_key: Option<String>,
}

fn default_repeat_min() -> usize {
    1
}
