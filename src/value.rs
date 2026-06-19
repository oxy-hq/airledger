//! `CellValue` — the typed-record cell representation.
//!
//! The Dart side uses `Object?` for every value flowing between the
//! form, the in-memory record, and the Sheets API. Rust needs a real
//! tagged union. This enum is that union: every value the engine
//! produces or consumes is one of these variants.
//!
//! Encoding/decoding to the wire (Sheets cell representation) happens
//! in [`crate::eval::codec`].

use chrono::{NaiveDate, NaiveDateTime};

/// One cell value — either an in-memory typed value or a wire-shaped
/// scalar. Designed so `CellValue::Null` is the right "empty" for both
/// optional form fields and Sheets blank cells.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

impl CellValue {
    /// True when this value represents "no value" — either an explicit
    /// `Null` or an empty `String`. Used by both the encoder (empty
    /// cells write `""`) and the form's required-field check.
    pub fn is_empty(&self) -> bool {
        match self {
            CellValue::Null => true,
            CellValue::String(s) => s.is_empty(),
            _ => false,
        }
    }

    /// Stringy display — what the value looks like when rendered as a
    /// plain string (titles, subtitles, history rows). Mirrors how the
    /// Dart side `.toString()`s `Object?` values.
    pub fn to_display_string(&self) -> String {
        match self {
            CellValue::Null => String::new(),
            CellValue::Bool(b) => b.to_string(),
            CellValue::Int(n) => n.to_string(),
            CellValue::Float(n) => n.to_string(),
            CellValue::String(s) => s.clone(),
            CellValue::Date(d) => d.format("%Y-%m-%d").to_string(),
            CellValue::DateTime(dt) => dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
        }
    }
}

/// One record — a row in a sheet, a fan-out batch entry, an entry the
/// form is composing. Mirrors `Map<String, Object?>` on the Dart side.
pub type Record = std::collections::BTreeMap<String, CellValue>;
