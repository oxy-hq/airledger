//! Overlay merge — the `.input.yml`'s top-level + per-field config
//! gets layered onto the bare `.view.yml`'s [`ViewSchema`] to produce
//! the final merged tree the app uses.
//!
//! The overlay is an intermediate parsed form. See `parse::input` for
//! the YAML side; this file is the post-parse merge.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::input::{
    InputSpec, ListDisplay, Plannable, PostLog, RepeatGroup,
};
use super::view::ViewSchema;

/// Parsed `.input.yml` before overlay-application. Carries the
/// view-level fields plus per-dim entries keyed by dim name.
#[derive(Debug, Clone, PartialEq)]
pub struct InputOverlay {
    /// Resolved from the file's `target:` field (basename of the
    /// paired `.view.yml`). Verified against the view's `name:` by
    /// [`apply_overlay`].
    pub view_name: String,
    pub date_field: Option<String>,
    pub plannable: Option<Plannable>,
    pub list_display: Option<ListDisplay>,
    pub spreadsheet_id: Option<String>,
    pub icon: Option<String>,
    pub post_log: Option<PostLog>,
    pub dimensions: BTreeMap<String, DimensionOverlay>,
    pub groups: BTreeMap<String, BTreeSet<String>>,
    pub top_metric: Option<String>,
    pub repeat_group: Option<RepeatGroup>,
}

/// Per-dim overlay — input spec + autocomplete samples + show_when +
/// derive. Each is optional.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct DimensionOverlay {
    pub input: Option<InputSpec>,
    pub samples: Option<Vec<String>>,
    pub show_when: Option<serde_yaml::Mapping>,
    pub derive: Option<super::view::Derive>,
}

/// Merge `overlay` into `view`. Returns a new [`ViewSchema`] with the
/// input-layer fields populated. Fails when:
/// - the overlay's `view_name` doesn't match `view.name`
/// - the overlay references a dim not declared on the view
/// - `top_metric` references a measure not declared on the view
pub fn apply_overlay(
    mut view: ViewSchema,
    overlay: InputOverlay,
) -> Result<ViewSchema, OverlayError> {
    if overlay.view_name != view.name {
        return Err(OverlayError::ViewNameMismatch {
            view_name: view.name.clone(),
            overlay_view_name: overlay.view_name,
        });
    }

    let declared: BTreeSet<&str> =
        view.dimensions.iter().map(|d| d.name.as_str()).collect();
    for k in overlay.dimensions.keys() {
        if !declared.contains(k.as_str()) {
            return Err(OverlayError::UnknownDimension {
                name: k.clone(),
                declared: view
                    .dimensions
                    .iter()
                    .map(|d| d.name.clone())
                    .collect(),
            });
        }
    }

    if let Some(ref m) = overlay.top_metric {
        let measure_names: BTreeSet<&str> =
            view.measures.iter().map(|x| x.name.as_str()).collect();
        if !measure_names.contains(m.as_str()) {
            return Err(OverlayError::UnknownTopMetric {
                name: m.clone(),
                declared: view
                    .measures
                    .iter()
                    .map(|x| x.name.clone())
                    .collect(),
            });
        }
    }

    for dim in &mut view.dimensions {
        if let Some(o) = overlay.dimensions.get(&dim.name) {
            dim.input = o.input.clone();
            if let Some(ref s) = o.samples {
                dim.samples = Some(s.clone());
            }
            if let Some(ref sw) = o.show_when {
                dim.show_when = Some(sw.clone());
            }
            if let Some(ref dv) = o.derive {
                dim.derive = Some(dv.clone());
            }
        }
    }

    view.date_field = overlay.date_field;
    view.spreadsheet_id = overlay.spreadsheet_id;
    view.list_display = overlay.list_display;
    view.plannable = overlay.plannable;
    view.icon = overlay.icon;
    view.post_log = overlay.post_log;
    view.groups = overlay.groups;
    view.top_metric = overlay.top_metric;
    view.has_input_overlay = true;
    view.repeat_group = overlay.repeat_group;

    Ok(view)
}

#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    #[error(
        ".input.yml view-name mismatch: {view_name} (.view.yml) != \
         {overlay_view_name} (.input.yml)"
    )]
    ViewNameMismatch {
        view_name: String,
        overlay_view_name: String,
    },
    #[error(
        ".input.yml references dimension \"{name}\" that is not \
         declared in .view.yml (declared: {declared:?})"
    )]
    UnknownDimension {
        name: String,
        declared: Vec<String>,
    },
    #[error(
        ".input.yml top_metric \"{name}\" is not declared as a \
         measure on .view.yml (declared: {declared:?})"
    )]
    UnknownTopMetric {
        name: String,
        declared: Vec<String>,
    },
}
