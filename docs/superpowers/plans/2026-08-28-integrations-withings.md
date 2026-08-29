# Integrations Framework + Withings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** External-source ingest into the local-first ledger — a generic engine `ledger_ingest` primitive plus the full Withings → weight pipeline with an Integrations page.

**Architecture:** Engine owns merge/fill rules, no-op idempotency, a local-only provenance table, and `deleted_dates` unwind — all inside one transaction per batch. The app owns sources: Integrations page, Withings OAuth (`flutter_web_auth_2` + `flutter_secure_storage`), `getmeas` pulls with a 90-day deletion reconcile, scheduled on the existing `SyncScheduler` triggers with a 6 h per-source floor. Spec: `docs/superpowers/specs/2026-08-28-integrations-withings-design.md`.

**Tech Stack:** Rust (rusqlite, serde), dart:ffi, Flutter (`flutter_web_auth_2`, `flutter_secure_storage`, `http`).

**Constraint:** Withings `client_id`/`client_secret` arrive late — config is wired end-to-end so the on-device OAuth test is the only step that needs them.

---

### Task 1: Store v2 — provenance table + accessors

**Files:**
- Modify: `src/store/db.rs`, `src/store/mod.rs`
- Test: `tests/store_unit.rs`

- [ ] **Step 1: Failing tests** (append to `tests/store_unit.rs`)

```rust
#[test]
fn provenance_round_trip_and_remove() {
    let store = temp_store("prov");
    use airledger_engine::store::Provenance;
    let written = rec(&[("body_fat_withing", CellValue::Float(18.2))]);
    store
        .provenance_set(&Provenance {
            view_name: "weight".into(),
            id: "row-1".into(),
            source: "withings".into(),
            fields: vec!["body_fat_withing".into()],
            written: written.clone(),
            created: false,
        })
        .unwrap();
    let p = store.provenance_get("weight", "row-1", "withings").unwrap().unwrap();
    assert_eq!(p.fields, vec!["body_fat_withing".to_string()]);
    assert_eq!(p.written, written);
    assert!(!p.created);
    store.provenance_remove("weight", "row-1", "withings").unwrap();
    assert!(store.provenance_get("weight", "row-1", "withings").unwrap().is_none());
}

#[test]
fn open_migrates_v1_store_to_v2() {
    let store = temp_store("migrate");
    assert_eq!(store.meta_get("schema_version").unwrap().as_deref(), Some("2"));
}
```

- [ ] **Step 2: Run** — `cargo test --test store_unit` → FAIL (no `Provenance`).

- [ ] **Step 3: Implement** — in `src/store/db.rs`:

Add to `Store::open`'s `execute_batch` DDL:

```sql
CREATE TABLE IF NOT EXISTS ingest_provenance (
  view_name TEXT NOT NULL,
  id        TEXT NOT NULL,
  source    TEXT NOT NULL,
  fields    TEXT NOT NULL,   -- JSON array: field names the source wrote
  written   TEXT NOT NULL,   -- JSON Record: the values as written
  created   INTEGER NOT NULL, -- 1 = the source created this row
  PRIMARY KEY (view_name, id, source)
);
```

Change the version seed to `'2'` and add after it (v1 → v2 upgrade is just the CREATE IF NOT EXISTS above, so bump unconditionally):

```rust
conn.execute("UPDATE meta SET value = '2' WHERE key = 'schema_version'", [])?;
```

New struct + methods (same file):

```rust
/// What one source wrote onto one row — the unwind anchor for
/// integration deletions. Local-only; never round-trips the Sheet.
#[derive(Debug, Clone)]
pub struct Provenance {
    pub view_name: String,
    pub id: String,
    pub source: String,
    pub fields: Vec<String>,
    pub written: Record,
    pub created: bool,
}

impl Store {
    pub fn provenance_set(&self, p: &Provenance) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO ingest_provenance(view_name, id, source, fields, written, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(view_name, id, source) DO UPDATE SET
               fields = excluded.fields, written = excluded.written,
               created = excluded.created",
            rusqlite::params![
                p.view_name, p.id, p.source,
                serde_json::to_string(&p.fields)?,
                serde_json::to_string(&p.written)?,
                p.created,
            ],
        )?;
        Ok(())
    }

    pub fn provenance_get(
        &self, view_name: &str, id: &str, source: &str,
    ) -> Result<Option<Provenance>, StoreError> {
        use rusqlite::OptionalExtension;
        let row = self.conn.query_row(
            "SELECT fields, written, created FROM ingest_provenance
             WHERE view_name = ?1 AND id = ?2 AND source = ?3",
            rusqlite::params![view_name, id, source],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)),
        ).optional()?;
        Ok(match row {
            None => None,
            Some((fields, written, created)) => Some(Provenance {
                view_name: view_name.into(), id: id.into(), source: source.into(),
                fields: serde_json::from_str(&fields)?,
                written: serde_json::from_str(&written)?,
                created,
            }),
        })
    }

    pub fn provenance_remove(
        &self, view_name: &str, id: &str, source: &str,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM ingest_provenance WHERE view_name = ?1 AND id = ?2 AND source = ?3",
            rusqlite::params![view_name, id, source],
        )?;
        Ok(())
    }
}
```

Export `Provenance` from `src/store/mod.rs` (`pub use db::{LocalRow, Provenance, Store};`).

- [ ] **Step 4: Run** — `cargo test --test store_unit` → PASS; full `cargo test` green.
- [ ] **Step 5: Commit** — `git commit -am "feat(store): v2 — ingest provenance table"`

---

### Task 2: Ingest core — create / fill-if-blank / owned-overwrite / no-op

**Files:**
- Create: `src/store/ingest.rs`
- Modify: `src/store/mod.rs`
- Test: `tests/ingest_unit.rs`

- [ ] **Step 1: Failing tests** — `tests/ingest_unit.rs`:

```rust
use airledger_engine::store::{ingest, IngestBatch, Store};
use airledger_engine::value::CellValue;
use airledger_engine::{apply_overlay, parse_input_overlay, parse_view};

fn weight_view() -> airledger_engine::ViewSchema {
    let base = parse_view(
        "name: weight\ndatasource: gsheets\ntable: weight\ndimensions:\n  - { name: id, type: string, expr: id }\n  - { name: date, type: date, expr: date }\n  - { name: time, type: string, expr: time }\n  - { name: weight_lbs, type: number, expr: weight_lbs }\n  - { name: body_fat_withing, type: number, expr: body_fat_withing }\n",
    ).unwrap();
    let overlay = parse_input_overlay("target: weight.view.yml\ndate_field: date\n").unwrap();
    apply_overlay(base, overlay).unwrap()
}

fn temp_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join("airledger-ingest-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}-{}.db", std::process::id()));
    std::fs::remove_file(&path).ok();
    Store::open(path.to_str().unwrap()).unwrap()
}

fn batch(json: &str) -> IngestBatch {
    serde_json::from_str(json).unwrap()
}

const DAY_BATCH: &str = r#"{
  "source": "withings",
  "owned_fields": ["body_fat_withing"],
  "fill_if_blank_fields": ["weight_lbs", "time"],
  "records": [{
    "date": {"kind":"date","value":"2026-08-28"},
    "time": {"kind":"string","value":"07:31"},
    "weight_lbs": {"kind":"float","value":180.9},
    "body_fat_withing": {"kind":"float","value":18.2}
  }]
}"#;

#[test]
fn creates_row_when_day_missing() {
    let store = temp_store("create");
    let view = weight_view();
    let res = ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (1, 0, 0));
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("body_fat_withing"), Some(&CellValue::Float(18.2)));
    assert!(!rows[0].get("id").unwrap().to_display_string().is_empty());
    assert_eq!(store.pending_count().unwrap(), 1, "created row is dirty → syncs");
}

#[test]
fn merges_into_existing_day_row_without_clobbering_manual_values() {
    let store = temp_store("merge");
    let view = weight_view();
    // Manual row: user weighed 180.5, no body fat.
    let mut manual = std::collections::BTreeMap::new();
    manual.insert("date".to_string(), CellValue::Date(
        chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()));
    manual.insert("weight_lbs".to_string(), CellValue::Float(180.5));
    store.create(&view, manual).unwrap();

    let res = ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 1, 0));
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(180.5)), "manual wins");
    assert_eq!(row.get("body_fat_withing"), Some(&CellValue::Float(18.2)), "owned written");
    assert_eq!(row.get("time"), Some(&CellValue::String("07:31".into())), "blank filled");
}

#[test]
fn replay_is_a_no_op_and_does_not_dirty() {
    let store = temp_store("replay");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    // Pretend sync ran: clear dirty.
    let row = store.list(&view, None).unwrap().remove(0);
    let id = row.get("id").unwrap().to_display_string();
    store.mark_synced(&view.name, &id, &row, Some(0)).unwrap();
    assert_eq!(store.pending_count().unwrap(), 0);

    let res = ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 0, 1));
    assert_eq!(store.pending_count().unwrap(), 0, "no-op must not re-dirty");
}

#[test]
fn owned_field_updates_when_source_value_changes() {
    let store = temp_store("owned");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    let changed = DAY_BATCH.replace("18.2", "17.9");
    let res = ingest(&store, &view, &batch(&changed)).unwrap();
    assert_eq!(res.updated, 1);
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("body_fat_withing"), Some(&CellValue::Float(17.9)));
}

#[test]
fn record_without_date_is_skipped() {
    let store = temp_store("nodate");
    let view = weight_view();
    let b = batch(r#"{"source":"withings","records":[{"weight_lbs":{"kind":"float","value":1.0}}]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.skipped, 1);
    assert!(store.list(&view, None).unwrap().is_empty());
}
```

- [ ] **Step 2: Run** — `cargo test --test ingest_unit` → compile FAIL.

- [ ] **Step 3: Implement** — `src/store/ingest.rs`:

```rust
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
        // Index live rows by their date display string.
        let rows = s.list(view, None)?;
        let mut by_date: BTreeMap<String, Record> = BTreeMap::new();
        for r in rows {
            let d = r.get(&date_field).map(|v| v.to_display_string()).unwrap_or_default();
            // First row of the day wins the match (one-row-per-day views).
            by_date.entry(d).or_insert(r);
        }

        for rec in &batch.records {
            let day = rec.get(&date_field).map(|v| v.to_display_string()).unwrap_or_default();
            if day.is_empty() {
                res.skipped += 1;
                continue;
            }
            match by_date.get(&day).cloned() {
                None => {
                    let created = s.create(view, rec.clone())?;
                    let id = created.get("id").map(|v| v.to_display_string()).unwrap_or_default();
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
                    let id = updated.get("id").map(|v| v.to_display_string()).unwrap_or_default();
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
    let _ = (s, view, batch, date_field, by_date, res); // Task 3
    Ok(())
}
```

`src/store/mod.rs`: `mod ingest;` + `pub use ingest::{ingest, IngestBatch, IngestResult};`.

- [ ] **Step 4: Run** — `cargo test --test ingest_unit` → PASS (deletion tests come next task); full suite green.
- [ ] **Step 5: Commit** — `git commit -am "feat(ingest): batch merge — create/fill/owned/no-op + provenance"`

---

### Task 3: Ingest deletions — `deleted_dates` unwind

**Files:**
- Modify: `src/store/ingest.rs`
- Test: `tests/ingest_unit.rs`

- [ ] **Step 1: Failing tests** (append):

```rust
#[test]
fn deleted_date_removes_source_created_untouched_row() {
    let store = temp_store("del-created");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.deleted, 1);
    assert!(store.list(&view, None).unwrap().is_empty());
}

#[test]
fn deleted_date_clears_only_source_fields_on_manual_row() {
    let store = temp_store("del-manual");
    let view = weight_view();
    let mut manual = std::collections::BTreeMap::new();
    manual.insert("date".to_string(), CellValue::Date(
        chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()));
    manual.insert("weight_lbs".to_string(), CellValue::Float(180.5));
    store.create(&view, manual).unwrap();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap(); // fills body_fat + time

    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.cleared, 1);
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(180.5)), "manual survives");
    assert!(row.get("body_fat_withing").map_or(true, |v| v.is_empty()), "owned cleared");
    assert!(row.get("time").map_or(true, |v| v.is_empty()), "filled field cleared");
}

#[test]
fn deleted_date_leaves_row_edited_after_ingest_but_clears_fields() {
    let store = temp_store("del-edited");
    let view = weight_view();
    ingest(&store, &view, &batch(DAY_BATCH)).unwrap();
    // User edits the source-created row afterwards.
    let mut row = store.list(&view, None).unwrap().remove(0);
    row.insert("weight_lbs".into(), CellValue::Float(181.0));
    store.update(&view, row).unwrap();

    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 1), "edited row must not be deleted");
    let row = &store.list(&view, None).unwrap()[0];
    assert_eq!(row.get("weight_lbs"), Some(&CellValue::Float(181.0)));
}

#[test]
fn deleted_date_without_provenance_is_ignored() {
    let store = temp_store("del-none");
    let view = weight_view();
    let mut manual = std::collections::BTreeMap::new();
    manual.insert("date".to_string(), CellValue::Date(
        chrono::NaiveDate::from_ymd_opt(2026, 8, 28).unwrap()));
    store.create(&view, manual).unwrap();
    let b = batch(r#"{"source":"withings","deleted_dates":["2026-08-28","2026-08-01"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 0));
    assert_eq!(store.list(&view, None).unwrap().len(), 1);
}
```

- [ ] **Step 2: Run** — first three FAIL.

- [ ] **Step 3: Implement** — replace the `apply_deletions` stub:

```rust
fn apply_deletions(
    s: &Store,
    view: &ViewSchema,
    batch: &IngestBatch,
    date_field: &str,
    by_date: &mut BTreeMap<String, Record>,
    res: &mut IngestResult,
) -> Result<(), StoreError> {
    let _ = date_field;
    for day in &batch.deleted_dates {
        let Some(row) = by_date.get(day).cloned() else { continue };
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
            let mut cleared = row.clone();
            for f in &prov.fields {
                cleared.insert(f.clone(), CellValue::Null);
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
```

Note the `created && untouched` comparison uses `prov.written` — for
source-created rows `written` is the full created record, so "untouched"
covers every field the source set at creation.

- [ ] **Step 4: Run** — `cargo test --test ingest_unit` → all PASS; full suite green.
- [ ] **Step 5: Commit** — `git commit -am "feat(ingest): deleted_dates unwind via provenance"`

---

### Task 4: FFI — `ledger_ingest`, `ledger_meta_get`, `ledger_meta_set`

**Files:**
- Modify: `src/ffi.rs`
- Test: `tests/ledger_ffi.rs`

- [ ] **Step 1: Failing test** (append to `tests/ledger_ffi.rs`; extend the `extern "C"` block with the three symbols, shaped like the existing ones — `ledger_ingest(handle, view_json, batch_json)`, `ledger_meta_get(handle, key)`, `ledger_meta_set(handle, key, value)`):

```rust
#[test]
fn ledger_ingest_and_meta_round_trip() {
    let dir = std::env::temp_dir().join("airledger-ledger-ffi");
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join(format!("ingest-{}.db", std::process::id()));
    std::fs::remove_file(&db).ok();
    unsafe {
        let db_path = CString::new(db.to_str().unwrap()).unwrap();
        let sid = CString::new("unused").unwrap();
        let sa = CString::new(FAKE_SA).unwrap();
        let mut err: *mut c_char = std::ptr::null_mut();
        let h = airledger_engine_ledger_open(db_path.as_ptr(), sid.as_ptr(), sa.as_ptr(), &mut err);
        assert!(!h.is_null());

        // View needs date_field for ingest → view JSON carries it directly.
        let view = CString::new(r#"{
            "name":"weight","datasource":"gsheets","table":"weight","date_field":"date",
            "dimensions":[
                {"name":"id","type":"string","expr":"id"},
                {"name":"date","type":"date","expr":"date"},
                {"name":"body_fat_withing","type":"number","expr":"body_fat_withing"}
            ]}"#).unwrap();
        let batch = CString::new(r#"{
            "source":"withings","owned_fields":["body_fat_withing"],
            "records":[{"date":{"kind":"date","value":"2026-08-28"},
                        "body_fat_withing":{"kind":"float","value":18.2}}]}"#).unwrap();
        let out = take_string(airledger_engine_ledger_ingest(h, view.as_ptr(), batch.as_ptr()));
        let res: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(res["created"], 1, "ingest: {out}");

        let key = CString::new("integration_cursor_withings").unwrap();
        let val = CString::new("1756400000").unwrap();
        let out = take_string(airledger_engine_ledger_meta_set(h, key.as_ptr(), val.as_ptr()));
        assert!(out.contains("ok"));
        let out = take_string(airledger_engine_ledger_meta_get(h, key.as_ptr()));
        let res: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(res["value"], "1756400000");

        airledger_engine_ledger_free_handle(h);
    }
    std::fs::remove_file(&db).ok();
}
```

- [ ] **Step 2: Run** — link FAIL (missing symbols).

- [ ] **Step 3: Implement** — append to `src/ffi.rs` (uses the existing `ledger_call` for ingest; meta ops lock the store directly like `ledger_pending`):

```rust
use crate::store::{ingest, IngestBatch};

/// Merge an externally-sourced batch into the local store. See
/// `store::ingest` for batch shape + rules. Returns the IngestResult
/// JSON (`{created, updated, unchanged, skipped, deleted, cleared}`).
///
/// # Safety
/// As [`airledger_engine_ledger_list`].
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_ingest(
    handle: *mut LedgerHandle,
    view_json_ptr: *const c_char,
    batch_json_ptr: *const c_char,
) -> *mut c_char {
    let batch: IngestBatch = match unsafe { c_str_to_str(batch_json_ptr) }
        .and_then(|s| serde_json::from_str(s).map_err(|e| format!("batch json: {e}")))
    {
        Ok(b) => b,
        Err(e) => return error_json(&e),
    };
    ledger_call(handle, view_json_ptr, |store, view| ingest(store, view, &batch))
}

/// Read a meta value: `{"value": "..."} ` or `{"value": null}`.
///
/// # Safety
/// `handle` valid; `key_ptr` nul-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_meta_get(
    handle: *mut LedgerHandle,
    key_ptr: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return error_json("null handle");
    }
    let key = match unsafe { c_str_to_str(key_ptr) } {
        Ok(k) => k,
        Err(e) => return error_json(&e),
    };
    let handle = unsafe { &*handle };
    let store = match handle.store.lock() {
        Ok(g) => g,
        Err(_) => return error_json("store mutex poisoned"),
    };
    match store.meta_get(key) {
        Ok(v) => result_json(&serde_json::json!({ "value": v })),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Write a meta value: `{"ok":true}`.
///
/// # Safety
/// As [`airledger_engine_ledger_meta_get`]; `value_ptr` also valid.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_meta_set(
    handle: *mut LedgerHandle,
    key_ptr: *const c_char,
    value_ptr: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return error_json("null handle");
    }
    let (key, value) = match (unsafe { c_str_to_str(key_ptr) }, unsafe { c_str_to_str(value_ptr) }) {
        (Ok(k), Ok(v)) => (k, v),
        (Err(e), _) | (_, Err(e)) => return error_json(&e),
    };
    let handle = unsafe { &*handle };
    let store = match handle.store.lock() {
        Ok(g) => g,
        Err(_) => return error_json("store mutex poisoned"),
    };
    match store.meta_set(key, value) {
        Ok(()) => result_json(&serde_json::json!({ "ok": true })),
        Err(e) => error_json(&e.to_string()),
    }
}
```

- [ ] **Step 4: Run** — `cargo test` → all green.
- [ ] **Step 5: Commit** — `git commit -am "feat(ffi): ledger_ingest + meta get/set"`

---

### Task 5: sdk-dart — ingest + meta on `EngineLedgerRepository`

**Files:**
- Modify: `sdk-dart/lib/src/bindings.dart`, `sdk-dart/lib/src/airledger_engine_base.dart`
- Test: `sdk-dart/test/ledger_test.dart`

- [ ] **Step 1: Bindings** — add lookups `ledgerIngest` (`_LedgerOpThreeNative`), `ledgerMetaGet` (`_LedgerOpTwoNative`), `ledgerMetaSet` (`_LedgerOpThreeNative`) for the three new symbols, following the existing ledger entries exactly (same typedefs already exist).

- [ ] **Step 2: Failing test** (append to `ledger_test.dart`, inside a new test using the same `fakeSa` setup):

```dart
  test('ingest merges a batch and meta round-trips', () async {
    final engine = AirledgerEngine.load();
    final dir = Directory.systemTemp.createTempSync('ingest');
    final ledger = engine.openLedger(
      dbPath: '${dir.path}/t.db',
      defaultSpreadsheetId: 'unused',
      serviceAccountJson: fakeSa,
    );
    final view = engine.parseViewPair(viewYaml: '''
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: date, expr: date }
  - { name: body_fat_withing, type: number, expr: body_fat_withing }
''', inputYaml: '''
target: weight.view.yml
date_field: date
''');
    final res = await ledger.ingest(view, {
      'source': 'withings',
      'owned_fields': ['body_fat_withing'],
      'records': [
        {
          'date': {'kind': 'date', 'value': '2026-08-28'},
          'body_fat_withing': {'kind': 'float', 'value': 18.2},
        }
      ],
    });
    expect(res['created'], 1);
    expect(await ledger.list(view), hasLength(1));

    await ledger.metaSet('integration_cursor_withings', '123');
    expect(await ledger.metaGet('integration_cursor_withings'), '123');
    expect(await ledger.metaGet('never_written'), isNull);
    ledger.close();
    dir.deleteSync(recursive: true);
  });
```

- [ ] **Step 3: Implement wrapper** — on `EngineLedgerRepository` (worker-isolate helpers copy the `_ledgerRunTwo`/`_ledgerRunSync` patterns; `ingest` reuses a three-arg runner with the batch JSON in place of the record):

```dart
  /// Merge an externally-sourced batch (see engine `IngestBatch`).
  /// Returns `{created, updated, unchanged, skipped, deleted, cleared}`.
  Future<Map<String, dynamic>> ingest(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> batchJson,
  ) async { ... same shape as _runTwo, calling b.ledgerIngest ... }

  /// Small per-ledger key/value store (integration cursors, status).
  Future<String?> metaGet(String key) async { ... b.ledgerMetaGet ... }
  Future<void> metaSet(String key, String value) async { ... b.ledgerMetaSet ... }
```

- [ ] **Step 4: Run** — `cd sdk-dart && dart test` → all PASS.
- [ ] **Step 5: Commit** — `git commit -am "feat(sdk-dart): ingest + meta on EngineLedgerRepository"`

---

### Task 6: Schema — `body_fat_withing` on the weight view

**Files:**
- Modify: `~/repos/airledger-fitness/views/weight.view.yml`, `~/repos/airledger-fitness/views/weight.input.yml`, `~/repos/ledger-schemas/views/weight.view.yml`

- [ ] **Step 1:** In both `weight.view.yml` files, after `body_fat_omron`:

```yaml
  - { name: body_fat_withing, type: number, expr: body_fat_withing, description: Body fat % from Withings scale }
```

In `weight.input.yml` `fields:` (check existing field style first):

```yaml
  body_fat_withing:
    widget: number
    min: 0
    max: 60
```

- [ ] **Step 2:** Validate: `cd ~/repos/airledger && cargo test --test parse_real_schemas` (fixtures may be copies — if the live repos aren't referenced, spot-parse with `parse_view` via a one-liner instead). Commit + push `airledger-fitness` (app pulls schemas from GitHub); commit `ledger-schemas`.

---

### Task 7: App config plumbing + deps

**Files:**
- Modify: `~/repos/airledger-archive/pubspec.yaml`, `lib/services/app_config.dart`, `android/app/src/main/AndroidManifest.xml`, `tool/brand.dart` (only if it filters config keys), `~/repos/airledger-fitness/ledger.yaml`

- [ ] **Step 1: Deps** — `pubspec.yaml`:

```yaml
  flutter_web_auth_2: ^4.0.0
  flutter_secure_storage: ^9.2.0
```

- [ ] **Step 2: Manifest** — inside `<application>`:

```xml
        <activity
            android:name="com.linusu.flutter_web_auth_2.CallbackActivity"
            android:exported="true">
            <intent-filter android:label="flutter_web_auth_2">
                <action android:name="android.intent.action.VIEW" />
                <category android:name="android.intent.category.DEFAULT" />
                <category android:name="android.intent.category.BROWSABLE" />
                <data android:scheme="airledger" android:host="oauth" />
            </intent-filter>
        </activity>
```

- [ ] **Step 3: Config** — `~/repos/airledger-fitness/ledger.yaml` gains (placeholders until the user's registration lands):

```yaml
integrations:
  withings:
    client_id: "SET_ME"
    client_secret: "SET_ME"
```

Read `tool/brand.dart` to see how `ledger.yaml` keys reach `assets/config.yaml`; pass `integrations:` through the same way. In `app_config.dart`, parse into:

```dart
class WithingsConfig {
  const WithingsConfig({required this.clientId, required this.clientSecret});
  final String clientId;
  final String clientSecret;
  bool get isConfigured => clientId.isNotEmpty && clientId != 'SET_ME';
}
```

exposed as `AppConfig.withings` (null when the block is absent).

- [ ] **Step 4:** `flutter pub get && flutter analyze` → no new errors. Commit both repos (do NOT push airledger-fitness until real creds are in — or push with SET_ME placeholders, which is safe).

---

### Task 8: WithingsIntegration — OAuth, pull, transform, reconcile

**Files:**
- Create: `lib/services/integrations/integration.dart`, `lib/services/integrations/withings.dart`
- Test: `test/withings_transform_test.dart`

- [ ] **Step 1: Interface** — `integration.dart`:

```dart
import 'package:flutter/widgets.dart';

/// One connectable external source. Implementations own auth, pull,
/// transform, and reconcile; they funnel everything through
/// `EngineLedgerRepository.ingest` and keep status in ledger meta
/// under `integration_<id>_*` keys.
abstract class Integration {
  String get id;                 // 'withings'
  String get displayName;        // 'Withings'
  String get targetDescription;  // '→ weight'
  bool get isConfigured;         // creds present in config

  Future<bool> get isConnected;
  Future<String?> get statusLine; // 'Connected · last pulled …' etc.

  Future<void> connect(BuildContext context);
  Future<void> disconnect();

  /// Pull new data. [force] ignores the 6 h interval;
  /// [fullReconcile] sweeps all history instead of the 90-day window.
  Future<void> pull({bool force = false, bool fullReconcile = false});
}
```

- [ ] **Step 2: Transform tests first** — `test/withings_transform_test.dart` against a pure function (no I/O):

```dart
import 'package:airledger/services/integrations/withings.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  final grps = [
    // Two weigh-ins on the same day — earliest must win.
    {
      'grpid': 1, 'date': 1756362660, // 2026-08-28 07:31 local
      'measures': [
        {'value': 82045, 'unit': -3, 'type': 1},  // 82.045 kg
        {'value': 182, 'unit': -1, 'type': 6},    // 18.2 %
      ],
    },
    {
      'grpid': 2, 'date': 1756399999,
      'measures': [{'value': 83000, 'unit': -3, 'type': 1}],
    },
  ];

  test('kg converts to lbs at 1dp, fat ratio passes through at 1dp', () {
    final recs = withingsGroupsToRecords(grps);
    expect(recs, hasLength(1), 'one record per day');
    final r = recs.first;
    expect(r['weight_lbs'], {'kind': 'float', 'value': 180.9}); // 82.045*2.20462
    expect(r['body_fat_withing'], {'kind': 'float', 'value': 18.2});
    expect((r['date'] as Map)['kind'], 'date');
    expect((r['time'] as Map)['kind'], 'string');
  });

  test('deletion set = provenance days minus current days', () {
    final deleted = withingsDeletedDates(
      windowDaysWithData: {'2026-08-28'},
      provenanceDays: {'2026-08-28', '2026-08-20'},
    );
    expect(deleted, ['2026-08-20']);
  });
}
```

- [ ] **Step 3: Implement `withings.dart`** — pure helpers + the class:

```dart
/// Transform Withings measuregrps → engine records. One record per
/// local day; the EARLIEST weigh-in of the day wins.
List<Map<String, dynamic>> withingsGroupsToRecords(List<dynamic> grps) { ... }

List<String> withingsDeletedDates({
  required Set<String> windowDaysWithData,
  required Set<String> provenanceDays,
}) { ... sorted difference ... }
```

Class internals:
- **connect**: `FlutterWebAuth2.authenticate(url: authorizeUrl, callbackUrlScheme: 'airledger')` where `authorizeUrl = https://account.withings.com/oauth2_user/authorize2?response_type=code&client_id=…&scope=user.metrics&redirect_uri=airledger://oauth/withings&state=<random>`; verify state; exchange at `https://wbsapi.withings.net/v2/oauth2` with `action=requesttoken&grant_type=authorization_code&…`; Withings wraps responses as `{status: 0, body: {...}}` — non-zero status is an error. Store `access_token`/`refresh_token`/`expiry` in `flutter_secure_storage` keys `withings_access`, `withings_refresh`, `withings_expiry`.
- **token refresh**: on each pull if expiry within 5 min → `grant_type=refresh_token`; failure → set meta `integration_withings_status = reconnect` and stop.
- **pull**: `POST https://wbsapi.withings.net/measure` with `action=getmeas&meastypes=1,6&category=1&lastupdate=<cursor from meta integration_cursor_withings, 0 first time>`; transform → ingest batch (`owned: [body_fat_withing]`, `fill_if_blank: [weight_lbs, time]`); THEN the reconcile window: second `getmeas` with `startdate/enddate` for the last 90 days (or epoch 0 when `fullReconcile`); build `windowDaysWithData`; `provenanceDays` come from a meta-tracked JSON set `integration_withings_days` that the source updates after every successful ingest (days it has contributed). `deleted_dates` = difference limited to the window; send a second ingest batch with only `deleted_dates`; update the days-set and cursor (max `modified`/`date` seen) in meta; set `integration_withings_last_pull` + status `ok`.
- **statusLine** reads the meta keys; **disconnect** wipes secure-storage tokens + cursor + status (leaves `integration_withings_days` so a reconnect stays consistent with provenance).
- **6 h floor**: `pull()` without `force` returns early if `integration_withings_last_pull` is younger than 6 h.

- [ ] **Step 4: Run** — `flutter test test/withings_transform_test.dart` → PASS; `flutter analyze` clean.
- [ ] **Step 5: Commit** — `git commit -am "feat(integrations): Withings source — oauth, pull, transform, reconcile"`

---

### Task 9: Integrations page + scheduling hookup

**Files:**
- Create: `lib/services/integrations/registry.dart`, `lib/ui/integrations_screen.dart`
- Modify: `lib/ui/home_screen.dart`, `lib/services/sync_scheduler.dart`

- [ ] **Step 1: Registry** — builds the list from config: `WithingsIntegration` when `AppConfig.withings != null`, plus a non-connectable `ComingSoonIntegration('Macrofactor', '→ meals (via Health Connect)')`. Singleton set up alongside `SyncScheduler.init`.

- [ ] **Step 2: Screen** — `IntegrationsScreen`: `ListView` of cards; each card renders name, `targetDescription`, `FutureBuilder` status line, and the action row (`Connect` / `Sync now` + overflow `Full reconcile`, `Disconnect`). Confirm dialog on disconnect. Unconfigured integration shows "Add client_id to ledger.yaml and rebrand" instead of Connect.

- [ ] **Step 3: Home entry** — in the home `ListView.separated` tail (where the Apps tile pattern lives), add an "Integrations" tile navigating to `IntegrationsScreen`.

- [ ] **Step 4: Scheduler** — `SyncScheduler.maybeSync` gains, before the ledger sync: `await IntegrationRegistry.instance?.pullDue();` (each integration's own 6 h floor + error containment inside `pull`; a source failure must not throw past the registry). `Sync now` on a card calls `pull(force: true)` then `SyncScheduler.instance?.maybeSync(manual: true)`.

- [ ] **Step 5:** `flutter analyze` clean, `flutter test` (pre-existing failures excepted). Commit — `git commit -am "feat(integrations): page, registry, scheduler hookup"`.

---

### Task 10: Build, ship, validate

- [ ] **Step 1:** Engine: `cargo test` + `./sdk-dart/scripts/build-android.sh`. Dart: `cd sdk-dart && dart test`.
- [ ] **Step 2:** `cd ~/repos/airledger-archive && dart run tool/brand.dart --config ~/repos/airledger-fitness/ledger.yaml` → installs on the Pixel.
- [ ] **Step 3: On-device (no creds needed):** Integrations page renders; Withings card shows the not-configured hint (or Connect, once creds land); weight view shows the new `body_fat_withing` field; sheet gains the column on next sync (no-op — already present).
- [ ] **Step 4: Blocked-on-creds checklist (run when client_id/secret arrive):** put real values in `ledger.yaml` → rebrand → Connect → browser consent → backfill lands (page shows N days synced; weight history shows scale data) → sheet rows appear after sync → delete a recent weigh-in in the Withings app → `Sync now` → row/fields unwind.
- [ ] **Step 5:** Docs: README phase note + port-plan entry; update memory. Final commits on all repos.

---

## Self-review (done at plan time)

- Spec coverage: §1 ingest → Tasks 2–4; provenance → Tasks 1, 3; §2 Withings → Task 8 (OAuth/pull/transform/backfill/reconcile), config → Task 7; §3 page + scheduling → Task 9; §4 schema → Task 6, errors woven through 8–9, testing per task + Task 10.
- Types consistent: `IngestBatch`/`IngestResult`/`Provenance` names match across Tasks 1–5; meta keys `integration_withings_*` consistent across 8–9.
- Known executor judgment calls: exact `flutter_web_auth_2`/`flutter_secure_storage` versions; brand.dart pass-through mechanics; Withings `modified` field availability for cursor (fall back to group `date` if absent).
