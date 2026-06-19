//! Cell encode/decode — mirrors `CellCodec` from
//! `airledger/lib/services/cell_codec.dart`.
//!
//! `encode` converts a typed [`CellValue`] into the wire-form value
//! the Sheets API stores (string, number, or bool). `decode` parses
//! the string the API hands back into a typed [`CellValue`] based on
//! the destination [`DimensionType`].

use chrono::{NaiveDate, NaiveDateTime};

use crate::schema::view::DimensionType;
use crate::value::CellValue;

/// Encode a value for the Sheets API. Returns a wire-shaped
/// [`CellValue`] (Null / Bool / Int / Float / String) — never the
/// typed Date / DateTime variants, which are projected to ISO
/// strings as the Sheets API stores them as text anyway.
pub fn encode(kind: DimensionType, value: &CellValue) -> CellValue {
    match value {
        // Blank cell — sent as empty string so the Sheets API doesn't
        // see a missing key and skip it on update.
        CellValue::Null => CellValue::String(String::new()),
        // Type-driven coercion.
        _ => match kind {
            DimensionType::String => CellValue::String(value.to_display_string()),
            DimensionType::Number => match value {
                CellValue::Int(n) => CellValue::Int(*n),
                CellValue::Float(n) => CellValue::Float(*n),
                CellValue::String(s) => parse_num(s).unwrap_or_else(|| {
                    CellValue::String(s.clone())
                }),
                CellValue::Bool(b) => CellValue::Int(if *b { 1 } else { 0 }),
                CellValue::Date(d) => {
                    CellValue::String(d.format("%Y-%m-%d").to_string())
                }
                CellValue::DateTime(dt) => CellValue::String(dt.to_string()),
                CellValue::Null => unreachable!(),
            },
            DimensionType::Date => match value {
                CellValue::Date(d) => {
                    CellValue::String(d.format("%Y-%m-%d").to_string())
                }
                CellValue::DateTime(dt) => CellValue::String(
                    dt.date().format("%Y-%m-%d").to_string(),
                ),
                _ => CellValue::String(value.to_display_string()),
            },
            DimensionType::Datetime => match value {
                CellValue::DateTime(dt) => {
                    CellValue::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
                }
                CellValue::Date(d) => CellValue::String(
                    d.format("%Y-%m-%dT00:00:00").to_string(),
                ),
                _ => CellValue::String(value.to_display_string()),
            },
            DimensionType::Boolean => match value {
                CellValue::Bool(b) => CellValue::Bool(*b),
                CellValue::String(s) => {
                    CellValue::Bool(s.eq_ignore_ascii_case("true"))
                }
                _ => CellValue::Bool(value.to_display_string()
                    .eq_ignore_ascii_case("true")),
            },
        },
    }
}

/// Decode a raw cell value (whatever the Sheets API returned — string
/// for the most part, since `valueInputOption: 'RAW'` keeps numbers as
/// numbers but reads come back stringified for everything else) into a
/// typed [`CellValue`] aligned to the destination dim.
pub fn decode(kind: DimensionType, raw: &str) -> CellValue {
    if raw.is_empty() {
        return CellValue::Null;
    }
    match kind {
        DimensionType::String => CellValue::String(raw.to_string()),
        DimensionType::Number => parse_num(raw).unwrap_or(CellValue::Int(0)),
        DimensionType::Date => parse_date(raw).unwrap_or(CellValue::Null),
        DimensionType::Datetime => parse_datetime(raw).unwrap_or(CellValue::Null),
        DimensionType::Boolean => CellValue::Bool(raw.eq_ignore_ascii_case("true")),
    }
}

fn parse_num(s: &str) -> Option<CellValue> {
    if let Ok(n) = s.parse::<i64>() {
        return Some(CellValue::Int(n));
    }
    if let Ok(n) = s.parse::<f64>() {
        return Some(CellValue::Float(n));
    }
    None
}

fn parse_date(s: &str) -> Option<CellValue> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .map(CellValue::Date)
}

fn parse_datetime(s: &str) -> Option<CellValue> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(CellValue::DateTime(dt));
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
        return Some(CellValue::DateTime(dt));
    }
    // Fall back to date-only — Sheets sometimes returns the date
    // portion when the cell was originally written as date.
    parse_date(s)
}
