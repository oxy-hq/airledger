//! The semantic layer — entities, dimensions, measures. What airlayer
//! and oxy expect. Mirrors the Dart `ViewSchema` from
//! `view_schema.dart`.

use serde::{Deserialize, Serialize};

use super::input::{InputSpec, ListDisplay, Plannable, PostLog, RepeatGroup};

/// One declared tracker — a `.view.yml` after overlay merge.
///
/// After [`crate::schema::overlay::apply_overlay`], every field on
/// this struct is populated from either the `.view.yml` (semantic) or
/// the `.input.yml` (UI). Before overlay merge, only the semantic
/// fields are set; UI fields stay at their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewSchema {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub datasource: String,
    pub table: String,

    /// Dim that holds the canonical date for each row. Used by the
    /// timeline date filter, the plannable workflow, and the history
    /// panel's date grouping. Set from the input overlay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_field: Option<String>,

    /// Per-view override for the gsheets workbook id. When None, the
    /// default `spreadsheet_id` from the app's runtime config applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spreadsheet_id: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<Entity>,

    pub dimensions: Vec<Dimension>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<Measure>,

    /// Title/subtitle template for the row tile in the timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_display: Option<ListDisplay>,

    /// "Plan then log" workflow: rows with [`Plannable::log_field`]
    /// blank are considered planned and surface a "Log now" action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plannable: Option<Plannable>,

    /// Lucide icon name, emoji, or URL — rendered next to the view
    /// title on the home screen tile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Optional post-log LLM hook. See [`PostLog`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_log: Option<PostLog>,

    /// Named value sets that show_when predicates reference via
    /// `in_group` / `not_in_group`. Each entry: `<name>` → set of
    /// values that share that label. Empty when no `groups:` block.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub groups: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,

    /// Name of a measure on this view used to score rows for the
    /// history panel's per-day "top" highlight. Null disables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_metric: Option<String>,

    /// True when this view has a paired `.input.yml` overlay. False
    /// for analytics-only views (e.g. body_composition over the weight
    /// table). Drives "show as a tappable tracker" decisions.
    #[serde(default)]
    pub has_input_overlay: bool,

    /// Declares that a subset of fields repeats together within a
    /// single form session (e.g. sauces' ingredient lines). See
    /// [`RepeatGroup`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_group: Option<RepeatGroup>,
}

impl ViewSchema {
    /// Lookup a dimension by its `name` (the canonical identifier
    /// other config references). Returns `None` if absent.
    pub fn dimension_by_name(&self, name: &str) -> Option<&Dimension> {
        self.dimensions.iter().find(|d| d.name == name)
    }

    /// Lookup a dimension by its `expr` (the sheet column header).
    /// Falls back to a `name`-match so callers don't have to specify
    /// expr for canonical names. Mirrors `dimensionByExpr` in Dart.
    pub fn dimension_by_expr(&self, expr: &str) -> Option<&Dimension> {
        self.dimensions
            .iter()
            .find(|d| d.expr == expr)
            .or_else(|| self.dimensions.iter().find(|d| d.name == expr))
    }

    /// Dimensions intended to appear on the entry form — `editable`
    /// true and no derive rule. Mirrors `editableDimensions` in Dart.
    pub fn editable_dimensions(&self) -> impl Iterator<Item = &Dimension> {
        self.dimensions
            .iter()
            .filter(|d| d.input.as_ref().map_or(true, |i| i.editable) && d.derive.is_none())
    }

    /// Dimensions with a `derive:` rule — auto-computed at save time
    /// and hidden from the form.
    pub fn derived_dimensions(&self) -> impl Iterator<Item = &Dimension> {
        self.dimensions.iter().filter(|d| d.derive.is_some())
    }
}

/// One row in `entities:`. Marks the primary key for the row grain,
/// or a foreign-key relationship to another table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: EntityKind,
    #[serde(default)]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Primary,
    Foreign,
}

/// One declared column. Semantic fields come from `.view.yml`;
/// `input`, `samples`, `show_when`, `derive` come from `.input.yml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dimension {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: DimensionType,
    /// The sheet column header (or SQL expression for analytics-only
    /// views). Defaults to `name` when omitted from the YAML.
    pub expr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Static autocomplete suggestions / dropdown options. From the
    /// input overlay (`.input.yml` field's `options:` or legacy
    /// `samples:`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<Vec<String>>,

    /// Input/UI config for this dim. None on analytics-only views.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InputSpec>,

    /// Auto-compute rule applied at save time. Mutually exclusive
    /// with `input` (derived dims are hidden from the form).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derive: Option<Derive>,

    /// Conditional visibility on the form. Each entry is
    /// `<other_field>: <predicate>`; all entries AND'd. See
    /// [`crate::schema::input::is_visible_given`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_when: Option<serde_yaml::Mapping>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DimensionType {
    String,
    Number,
    Date,
    Datetime,
    Boolean,
}

/// Per-dim derive rule — take the value of `from` and run it through
/// `format` at save time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Derive {
    pub from: String,
    pub format: DeriveFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeriveFormat {
    WeekdayLong,
    WeekdayShort,
    IsoDate,
    IsoDateTime,
}

/// One declared aggregate. Identical shape across both layers (no
/// input-side fields on measures).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: MeasureType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Mirrors airlayer's measure types. `custom` and `number` are
/// "passthrough" — the `expr` is emitted verbatim into SQL with no
/// aggregation wrapper, so the schema author can write
/// `STDDEV_SAMP(weight_lbs)` or a constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureType {
    Count,
    Sum,
    Average,
    Max,
    Min,
    CountDistinct,
    Custom,
    Number,
}
