//! Record-level helpers shared by the sheets repo and the local store.

use chrono::{NaiveDate, NaiveTime};

use crate::schema::view::ViewSchema;
use crate::value::{CellValue, Record};

/// Date-filter + chronological sort, exactly as the app expects from
/// `list`: rows on `on_date` (when the view has a `date_field`),
/// ordered by the plannable log field's time-of-day, empty times
/// last. Without a date filter (or date_field) records pass through
/// unchanged. Sort is stable, so callers' input order is the
/// tiebreak (sheet order / sort_key order).
pub fn filter_and_sort(
    view: &ViewSchema,
    records: Vec<Record>,
    on_date: Option<NaiveDate>,
) -> Vec<Record> {
    let Some(on_date) = on_date else {
        return records;
    };
    let Some(date_field) = view.date_field.clone() else {
        return records;
    };

    let mut filtered: Vec<Record> = records
        .into_iter()
        .filter(|r| matches!(r.get(&date_field), Some(CellValue::Date(d)) if *d == on_date))
        .collect();

    let Some(log_field) = view.plannable.as_ref().map(|p| p.log_field.clone()) else {
        return filtered;
    };
    filtered.sort_by(|a, b| {
        let av = a
            .get(&log_field)
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        let bv = b
            .get(&log_field)
            .map(|v| v.to_display_string())
            .unwrap_or_default();
        match (av.is_empty(), bv.is_empty()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => match (parse_time(&av), parse_time(&bv)) {
                (Some(a), Some(b)) => a.cmp(&b),
                _ => av.cmp(&bv),
            },
        }
    });
    filtered
}

/// Parse a time-of-day string for chronological sort. 12-hour forms
/// first, then 24-hour — the formats the Dart `_parseTime` recognized.
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    for fmt in &[
        "%-I:%M:%S %p",
        "%-I:%M %p",
        "%I:%M:%S %p",
        "%I:%M %p",
        "%H:%M:%S",
        "%H:%M",
    ] {
        if let Ok(t) = NaiveTime::parse_from_str(s, fmt) {
            return Some(t);
        }
    }
    None
}
