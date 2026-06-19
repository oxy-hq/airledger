//! airledger-engine — the shared ingest engine, port of
//! `airledger/lib/services/` + `airledger/lib/models/` business logic
//! from Dart to Rust.
//!
//! Phase 1: the schema model + the paired-file YAML parser. Same
//! tree shape the Dart side produces, deterministically derivable
//! from the same input.

pub mod schema;
pub mod parse;

pub use parse::{parse_input_overlay, parse_view, ParseError};
pub use schema::{
    apply_overlay, OverlayError, ViewSchema,
};
