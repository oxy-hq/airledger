//! airledger-engine — the shared ingest engine, port of
//! `airledger/lib/services/` + `airledger/lib/models/` business logic
//! from Dart to Rust.
//!
//! Phase 1: the schema model + the paired-file YAML parser. Same
//! tree shape the Dart side produces, deterministically derivable
//! from the same input.

pub mod eval;
pub mod ffi;
pub mod parse;
pub mod records;
pub mod schema;
pub mod sheets;
pub mod store;
pub mod value;

pub use eval::{
    apply_derives, decode, encode, is_visible_given, run_derive,
    TemplateInterpolator,
};
pub use parse::{parse_input_overlay, parse_view, ParseError};
pub use schema::{apply_overlay, OverlayError, ViewSchema};
pub use sheets::{shift_row_indexes, ServiceAccount, SheetsError, SheetsRepository, ROW_INDEX_KEY};
pub use value::{CellValue, Record};
