//! `ledger_ingest` — merge externally-sourced records into the local
//! store. Owns the correctness rules every integration shares:
//! match-by-date, owned vs fill-if-blank fields, no-op idempotency,
//! provenance bookkeeping, and deletion unwind. One transaction per
//! batch; ingested changes land dirty so the ordinary sync pushes
//! them to the Sheet.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::view::ViewSchema;
use crate::value::{CellValue, Record};

use super::{Provenance, Store, StoreError};

#[derive(Debug, Deserialize)]
pub struct IngestBatch {
    pub source: String,
    #[serde(default)]
    pub owned_fields: Vec<String>,
    #[serde(default)]
    pub fill_if_blank_fields: Vec<String>,
    #[serde(default)]
    pub records: Vec<Record>,
    #[serde(default)]
    pub deleted_dates: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct IngestResult {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub deleted: usize,
    pub cleared: usize,
}

/// Apply one batch. Requires the view to declare a `date_field`.
pub fn ingest(
    store: &Store,
    view: &ViewSchema,
    batch: &IngestBatch,
) -> Result<IngestResult, StoreError> {
    let date_field = view
        .date_field
        .clone()
        .ok_or_else(|| StoreError::NotFound("date_field".into(), view.name.clone()))?;
    store.tx(|s| {
        let mut res = IngestResult::default();
        // Index live rows by their date display string. First row of
        // the day wins the match (one-row-per-day views).
        let rows = s.list(view, None)?;
        let mut by_date: BTreeMap<String, Record> = BTreeMap::new();
        for r in rows {
            let d = r
                .get(&date_field)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            by_date.entry(d).or_insert(r);
        }

        for rec in &batch.records {
            let day = rec
                .get(&date_field)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            if day.is_empty() {
                res.skipped += 1;
                continue;
            }
            match by_date.get(&day).cloned() {
                None => {
                    let created = s.create(view, rec.clone())?;
                    let id = created
                        .get("id")
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();
                    s.provenance_set(&Provenance {
                        view_name: view.name.clone(),
                        id,
                        source: batch.source.clone(),
                        fields: rec.keys().filter(|k| *k != "id").cloned().collect(),
                        written: created.clone(),
                        created: true,
                    })?;
                    by_date.insert(day, created);
                    res.created += 1;
                }
                Some(existing) => {
                    let mut updated = existing.clone();
                    let mut wrote: Vec<String> = Vec::new();
                    for f in &batch.owned_fields {
                        if let Some(v) = rec.get(f) {
                            if updated.get(f) != Some(v) {
                                updated.insert(f.clone(), v.clone());
                            }
                            wrote.push(f.clone());
                        }
                    }
                    for f in &batch.fill_if_blank_fields {
                        if let Some(v) = rec.get(f) {
                            let blank = updated.get(f).map_or(true, |cur| cur.is_empty());
                            if blank {
                                updated.insert(f.clone(), v.clone());
                                wrote.push(f.clone());
                            }
                        }
                    }
                    if updated == existing {
                        res.unchanged += 1;
                        continue;
                    }
                    s.update(view, updated.clone())?;
                    let id = updated
                        .get("id")
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();
                    let mut written = Record::new();
                    for f in &wrote {
                        if let Some(v) = updated.get(f) {
                            written.insert(f.clone(), v.clone());
                        }
                    }
                    s.provenance_set(&Provenance {
                        view_name: view.name.clone(),
                        id,
                        source: batch.source.clone(),
                        fields: wrote,
                        written,
                        created: false,
                    })?;
                    by_date.insert(day, updated);
                    res.updated += 1;
                }
            }
        }

        apply_deletions(s, view, batch, &date_field, &mut by_date, &mut res)?;
        Ok(res)
    })
}

fn apply_deletions(
    s: &Store,
    view: &ViewSchema,
    batch: &IngestBatch,
    date_field: &str,
    by_date: &mut BTreeMap<String, Record>,
    res: &mut IngestResult,
) -> Result<(), StoreError> {
    for day in &batch.deleted_dates {
        let Some(row) = by_date.get(day).cloned() else {
            continue;
        };
        let id = row.get("id").map(|v| v.to_display_string()).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let Some(prov) = s.provenance_get(&view.name, &id, &batch.source)? else {
            continue; // the source never touched this day
        };
        // "Untouched since": every field the source wrote still holds
        // the value the source wrote.
        let untouched = prov
            .fields
            .iter()
            .all(|f| row.get(f) == prov.written.get(f));
        if prov.created && untouched {
            s.delete(view, &row)?; // tombstone → sync removes the sheet row
            by_date.remove(day);
            res.deleted += 1;
        } else {
            // Clear only fields still holding the source's value —
            // user edits to a source-written field survive, and the
            // date field is exempt (it's the row's identity).
            let mut cleared = row.clone();
            for f in &prov.fields {
                if f == date_field {
                    continue;
                }
                if row.get(f) == prov.written.get(f) {
                    cleared.insert(f.clone(), CellValue::Null);
                }
            }
            if cleared != row {
                s.update(view, cleared.clone())?;
                by_date.insert(day.clone(), cleared);
                res.cleared += 1;
            }
        }
        s.provenance_remove(&view.name, &id, &batch.source)?;
    }
    Ok(())
}
