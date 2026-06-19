//! Derived-field computation — mirrors `derive.dart`. Runs at
//! save time over any dim with a `derive:` rule.

use chrono::{Datelike, NaiveDate, NaiveDateTime};

use crate::schema::view::{Derive, DeriveFormat, ViewSchema};
use crate::value::{CellValue, Record};

/// Apply one [`Derive`] to a source value. Returns `None` when the
/// source is missing or of the wrong type for the requested format.
pub fn run_derive(derive: &Derive, source: &CellValue) -> Option<CellValue> {
    let date = source_as_date(source)?;
    Some(match derive.format {
        DeriveFormat::WeekdayLong => {
            CellValue::String(format_weekday_long(date))
        }
        DeriveFormat::WeekdayShort => {
            CellValue::String(format_weekday_short(date))
        }
        DeriveFormat::IsoDate => {
            CellValue::String(date.format("%Y-%m-%d").to_string())
        }
        DeriveFormat::IsoDateTime => match source {
            CellValue::DateTime(dt) => {
                CellValue::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string())
            }
            _ => CellValue::String(
                NaiveDateTime::new(
                    date,
                    chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
                )
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string(),
            ),
        },
    })
}

/// Walk `view.derived_dimensions()` and fill in any derived value
/// that isn't already populated on the record. Mirrors `applyDerives`.
pub fn apply_derives(view: &ViewSchema, record: &mut Record) {
    let derives: Vec<(String, Derive)> = view
        .derived_dimensions()
        .filter_map(|d| d.derive.as_ref().map(|dv| (d.name.clone(), dv.clone())))
        .collect();
    for (name, derive) in derives {
        if record.get(&name).is_some_and(|v| !v.is_empty()) {
            continue;
        }
        let Some(source) = record.get(&derive.from).cloned() else {
            continue;
        };
        if let Some(derived) = run_derive(&derive, &source) {
            record.insert(name, derived);
        }
    }
}

fn source_as_date(v: &CellValue) -> Option<NaiveDate> {
    match v {
        CellValue::Date(d) => Some(*d),
        CellValue::DateTime(dt) => Some(dt.date()),
        CellValue::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").ok(),
        _ => None,
    }
}

fn format_weekday_long(d: NaiveDate) -> String {
    match d.weekday() {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
    .to_string()
}

fn format_weekday_short(d: NaiveDate) -> String {
    match d.weekday() {
        chrono::Weekday::Mon => "Mon",
        chrono::Weekday::Tue => "Tue",
        chrono::Weekday::Wed => "Wed",
        chrono::Weekday::Thu => "Thu",
        chrono::Weekday::Fri => "Fri",
        chrono::Weekday::Sat => "Sat",
        chrono::Weekday::Sun => "Sun",
    }
    .to_string()
}
