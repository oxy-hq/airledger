//! YAML → typed-schema parsers.
//!
//! Two entry points mirror the Dart side:
//! - [`parse_view`] for `.view.yml` (the semantic layer).
//! - [`parse_input_overlay`] for `.input.yml` (the UI overlay).
//!
//! Once both are parsed, merge them with
//! [`crate::schema::apply_overlay`].

pub mod view;
pub mod input;

pub use view::parse_view;
pub use input::parse_input_overlay;

/// Top-level parser error. Each parser uses this for surface-level
/// failures; [`crate::schema::OverlayError`] is returned by the
/// merge step.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("schema error: {0}")]
    Schema(String),
}
