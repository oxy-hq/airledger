//! `ledger_ingest` — merge externally-sourced records into the local
//! store. Owns the correctness rules every integration shares:
//! rows match by `date_field` (day-grained sources) or by a
//! configurable `match_field` (row-grained sources, e.g. one row per
//! ascent), owned vs fill-if-blank fields (fill when blank, or revise
//! the source's own unedited value), no-op idempotency, provenance
//! bookkeeping, and deletion unwind. One transaction per batch;
//! ingested changes land dirty so the ordinary sync pushes them to
//! the Sheet.

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
    /// When set, rows match records by this dimension's value instead of
    /// by `date_field` — row-grained sources (one row per ascent) rather
    /// than day-grained ones. Rows with an empty value for the field are
    /// invisible to the batch (hand-entered rows are never touched).
    #[serde(default)]
    pub match_field: Option<String>,
    /// Unwind list for `match_field` mode (values of that field). Used
    /// instead of `deleted_dates` when `match_field` is set.
    #[serde(default)]
    pub deleted_ids: Vec<String>,
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
    if let Some(mf) = &batch.match_field {
        if !view.dimensions.iter().any(|d| &d.name == mf) {
            return Err(StoreError::NotFound(format!("match_field {mf}"), view.name.clone()));
        }
    }
    // In match_field mode, rows are keyed by the given dimension's value
    // rather than by date (row-grained sources: many rows per day).
    // In date mode, key_field == date_field, preserving all prior behavior.
    let key_field = batch.match_field.clone().unwrap_or_else(|| date_field.clone());
    store.tx(|s| {
        let mut res = IngestResult::default();
        // Index live rows by their key display string. In date mode this
        // is the date; in match_field mode it is the match dimension value.
        // First row wins for any duplicate key (one-row-per-key invariant).
        // Empty-key rows are invisible to the batch — hand-entered rows
        // without a kaya_id are never matched or overwritten.
        //
        // Note: empty-date rows used to be indexed under ""; records with
        // an empty date were skipped and deleted_dates never contains "",
        // so skipping empty keys is observably identical in date mode.
        let rows = s.list(view, None)?;
        let mut by_key: BTreeMap<String, Record> = BTreeMap::new();
        for r in rows {
            let k = r
                .get(&key_field)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            if k.is_empty() {
                continue;
            }
            by_key.entry(k).or_insert(r);
        }

        for rec in &batch.records {
            let key = rec
                .get(&key_field)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            if key.is_empty() {
                res.skipped += 1;
                continue;
            }
            match by_key.get(&key).cloned() {
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
                    by_key.insert(key, created);
                    res.created += 1;
                }
                Some(existing) => {
                    let existing_id = existing
                        .get("id")
                        .map(|v| v.to_display_string())
                        .unwrap_or_default();
                    let prov =
                        s.provenance_get(&view.name, &existing_id, &batch.source)?;
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
                            // The cell still holds exactly what this
                            // source last wrote — revising it corrects
                            // the source's own value, not a user edit.
                            let sources_own = prov.as_ref().is_some_and(|p| {
                                p.fields.iter().any(|pf| pf == f)
                                    && p.written.get(f) == updated.get(f)
                            });
                            if blank || sources_own {
                                if updated.get(f) != Some(v) {
                                    updated.insert(f.clone(), v.clone());
                                }
                                wrote.push(f.clone());
                            }
                        }
                    }
                    if updated == existing {
                        res.unchanged += 1;
                        continue;
                    }
                    s.update(view, updated.clone())?;
                    // Merge into any existing provenance: fields the
                    // source wrote earlier stay owned, and `created`
                    // survives so deletion unwind still removes whole
                    // source-created rows.
                    let mut fields =
                        prov.as_ref().map(|p| p.fields.clone()).unwrap_or_default();
                    let mut written =
                        prov.as_ref().map(|p| p.written.clone()).unwrap_or_else(Record::new);
                    for f in &wrote {
                        if !fields.contains(f) {
                            fields.push(f.clone());
                        }
                        if let Some(v) = updated.get(f) {
                            written.insert(f.clone(), v.clone());
                        }
                    }
                    s.provenance_set(&Provenance {
                        view_name: view.name.clone(),
                        id: existing_id,
                        source: batch.source.clone(),
                        fields,
                        written,
                        created: prov.as_ref().is_some_and(|p| p.created),
                    })?;
                    by_key.insert(key, updated);
                    res.updated += 1;
                }
            }
        }

        // In match_field mode the unwind list is deleted_ids; the index
        // is keyed by that field, so deleted_dates would be meaningless
        // (and vice versa).
        let unwind: &[String] = if batch.match_field.is_some() {
            &batch.deleted_ids
        } else {
            &batch.deleted_dates
        };
        apply_deletions(s, view, &batch.source, &date_field, &key_field, unwind, &mut by_key, &mut res)?;
        Ok(res)
    })
}

#[allow(clippy::too_many_arguments)]
fn apply_deletions(
    s: &Store,
    view: &ViewSchema,
    source: &str,
    date_field: &str,
    key_field: &str,
    unwind: &[String],
    by_key: &mut BTreeMap<String, Record>,
    res: &mut IngestResult,
) -> Result<(), StoreError> {
    for key in unwind {
        let Some(row) = by_key.get(key).cloned() else {
            continue;
        };
        let id = row.get("id").map(|v| v.to_display_string()).unwrap_or_default();
        if id.is_empty() {
            continue;
        }
        let Some(prov) = s.provenance_get(&view.name, &id, source)? else {
            continue; // the source never touched this row
        };
        // "Untouched since": every field the source wrote still holds
        // the value the source wrote.
        let untouched = prov
            .fields
            .iter()
            .all(|f| row.get(f) == prov.written.get(f));
        if prov.created && untouched {
            s.delete(view, &row)?; // tombstone → sync removes the sheet row
            by_key.remove(key);
            res.deleted += 1;
        } else {
            // Clear only fields still holding the source's value —
            // user edits to a source-written field survive, and both
            // identity fields are exempt (date_field and key_field).
            let mut cleared = row.clone();
            for f in &prov.fields {
                if f == date_field || f == key_field {
                    continue; // row identity fields survive the clear
                }
                if row.get(f) == prov.written.get(f) {
                    cleared.insert(f.clone(), CellValue::Null);
                }
            }
            if cleared != row {
                s.update(view, cleared.clone())?;
                by_key.insert(key.clone(), cleared);
                res.cleared += 1;
            }
        }
        s.provenance_remove(&view.name, &id, source)?;
    }
    Ok(())
}
