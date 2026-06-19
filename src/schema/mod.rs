//! The schema model — pure Rust types mirroring the Dart `ViewSchema`
//! tree in `airledger/lib/models/view_schema.dart`.
//!
//! Two layers, one tree:
//! - **View layer** (semantic): entities, dimensions, measures, table,
//!   datasource. Whatever airlayer / oxy can parse without UI noise.
//! - **Input layer** (UI/ingest): widgets, defaults, derive,
//!   show_when, groups, plannable, list_display, repeat_group,
//!   top_metric. Owned by airledger; never seen by analytical
//!   consumers.
//!
//! The two layers come from paired files (`<name>.view.yml` and
//! `<name>.input.yml`) and are merged into a single [`ViewSchema`]
//! via [`apply_overlay`].

pub mod view;
pub mod input;
pub mod overlay;

pub use view::*;
pub use input::*;
pub use overlay::*;
