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
use serde::{Deserialize, Serialize};

/// One cell value — either an in-memory typed value or a wire-shaped
/// scalar. Designed so `CellValue::Null` is the right "empty" for both
/// optional form fields and Sheets blank cells.
///
/// Serializes to a tagged JSON envelope: `{"kind":"int","value":42}`,
/// `{"kind":"date","value":"2026-06-19"}`, `{"kind":"null"}`. The
/// envelope lets the Dart side recover `DateTime` vs string for
/// the date variants without consulting the schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
#[serde(rename_all = "snake_case")]
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

    /// Value equality across wire representations. The Sheets round-
    /// trip is lossy about NUMERIC SHAPE — "26" decodes as `Int(26)`
    /// while sources send `Float(26.0)` — so anything that decides
    /// "did this value actually change?" (ingest merges, provenance
    /// untouched-checks) must use this, not `==`. Everything else
    /// falls back to plain equality.
    pub fn equivalent(&self, other: &CellValue) -> bool {
        match (self, other) {
            (CellValue::Int(a), CellValue::Float(b))
            | (CellValue::Float(b), CellValue::Int(a)) => *a as f64 == *b,
            _ => self == other,
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

/// Record equality across wire representations — the record-level
/// counterpart of [`CellValue::equivalent`]. Two extra tolerances the
/// sheet round-trip demands:
/// - numeric shape (`Int(26)` vs `Float(26.0)`),
/// - a MISSING key equals an explicit `Null` (pulled rows carry a key
///   for every mapped column, blank cells included; locally-composed
///   rows simply omit fields that were never set).
///
/// The sync merge uses this to decide "did the remote actually
/// change?" — plain `==` manufactured phantom remote edits after
/// every push, and phantom diffs are what fed the 429 push storm.
pub fn records_equivalent(a: &Record, b: &Record) -> bool {
    let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter().all(|k| {
        let av = a.get(k).unwrap_or(&CellValue::Null);
        let bv = b.get(k).unwrap_or(&CellValue::Null);
        av.equivalent(bv)
    })
}
