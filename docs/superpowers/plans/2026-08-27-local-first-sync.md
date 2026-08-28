# Local-First Storage + Wifi-Gated Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the local SQLite store the app's source of truth with bidirectional, app-wins sync to Google Sheets, exposed through the engine's FFI + sdk-dart.

**Architecture:** New `store` module (rusqlite, generic `rows` table keyed by `(view_name, id)` with `base`/`dirty`/`deleted` sync metadata) + new `sync` module (pure three-way merge → actions, executed against a `SyncRemote` trait implemented by `SheetsRepository`). FFI grows a `LedgerHandle` mirroring the sheets handle; sdk-dart grows `EngineLedgerRepository`. Spec: `docs/superpowers/specs/2026-08-27-local-first-sync-design.md`.

**Tech Stack:** Rust (rusqlite bundled, serde_json, chrono, uuid), existing sheets module, dart:ffi.

**Scope note:** The archive Flutter app repo is NOT on this machine. App wiring (`SyncScheduler`, settings toggle, connector swap) is documented as follow-up in Task 10, not implemented here.

---

### Task 1: Store module — open + migration

**Files:**
- Modify: `Cargo.toml` (add rusqlite)
- Create: `src/store/mod.rs`, `src/store/db.rs`
- Modify: `src/lib.rs` (export)
- Test: `tests/store_unit.rs`

- [x] **Step 1: Add dependency**

In `Cargo.toml` `[dependencies]`:

```toml
# Local-first store. `bundled` compiles SQLite from source so Android
# NDK / iOS cross-builds don't need a system libsqlite3.
rusqlite = { version = "0.32", features = ["bundled"] }
```

- [x] **Step 2: Write failing test**

`tests/store_unit.rs`:

```rust
use airledger_engine::store::Store;

#[test]
fn open_creates_schema_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("airledger-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t1.db");
    let path_str = path.to_str().unwrap();
    {
        let store = Store::open(path_str).expect("first open");
        assert_eq!(store.meta_get("schema_version").unwrap().as_deref(), Some("1"));
    }
    // Reopen: no error, version unchanged.
    let store = Store::open(path_str).expect("reopen");
    assert_eq!(store.meta_get("schema_version").unwrap().as_deref(), Some("1"));
    std::fs::remove_file(&path).ok();
}
```

- [x] **Step 3: Run to verify failure** — `cargo test --test store_unit` → compile error: no `store` module.

- [x] **Step 4: Implement**

`src/store/mod.rs`:

```rust
//! Local-first store — SQLite source of truth for ledger rows.
//!
//! One generic `rows` table holds every view's records (schemas are
//! dynamic YAML, so no per-view DDL). Sync metadata rides alongside:
//! `base` (remote copy as of last sync — the three-way-merge anchor),
//! `dirty` (local change not yet pushed), `deleted` (tombstone).

mod db;

use thiserror::Error;

pub use db::{LocalRow, Store};

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("record json: {0}")]
    RecordJson(#[from] serde_json::Error),
    #[error("no row with id=\"{0}\" in \"{1}\"")]
    NotFound(String, String),
    #[error("record has no id — the store addresses rows by id only")]
    NoId,
}
```

`src/store/db.rs` (this task: struct, open, meta):

```rust
use rusqlite::Connection;

use crate::value::Record;

use super::StoreError;

/// One local row with its sync metadata — the shape the sync engine
/// consumes. `base` is `None` for never-synced (locally new) rows.
#[derive(Debug, Clone)]
pub struct LocalRow {
    pub id: String,
    pub data: Record,
    pub base: Option<Record>,
    pub dirty: bool,
    pub deleted: bool,
}

/// SQLite-backed local store. Synchronous like the sheets module —
/// Dart consumers call from a worker isolate.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating if needed) the store at `path`. WAL so a
    /// mid-write kill can't corrupt. `schema_version` in `meta`
    /// guards future migrations.
    pub fn open(path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS rows (
               view_name  TEXT NOT NULL,
               id         TEXT NOT NULL,
               data       TEXT NOT NULL,
               base       TEXT,
               dirty      INTEGER NOT NULL DEFAULT 0,
               deleted    INTEGER NOT NULL DEFAULT 0,
               sort_key   INTEGER NOT NULL DEFAULT 0,
               updated_at TEXT NOT NULL,
               PRIMARY KEY (view_name, id)
             );
             CREATE TABLE IF NOT EXISTS meta (
               key   TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );",
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', '1')",
            [],
        )?;
        Ok(Self { conn })
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        use rusqlite::OptionalExtension;
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }
}
```

In `src/lib.rs` add `pub mod store;` alongside the existing module declarations.

`sort_key` note: mirrors sheet row order (ascending = top-of-sheet first). Local creates take `MIN(sort_key) - 1` (newest-first, like the sheet's insert-at-row-2); sync rewrites it from remote row indexes.

- [x] **Step 5: Run test** — `cargo test --test store_unit` → PASS. Also `cargo test` (all existing suites still green).

- [x] **Step 6: Commit** — `git add -A && git commit -m "feat(store): SQLite store skeleton — open, migration, meta"`

---

### Task 2: Store CRUD + pending count

**Files:**
- Modify: `src/store/db.rs`
- Test: `tests/store_unit.rs`

- [x] **Step 1: Write failing tests** (append to `tests/store_unit.rs`)

```rust
use airledger_engine::parse_view;
use airledger_engine::value::CellValue;
use std::collections::BTreeMap;

const WEIGHT_VIEW: &str = r#"
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: date, expr: date }
  - { name: weight_lbs, type: number, expr: weight_lbs }
"#;

fn temp_store(tag: &str) -> Store {
    let dir = std::env::temp_dir().join("airledger-store-tests");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{tag}-{}.db", std::process::id()));
    std::fs::remove_file(&path).ok();
    Store::open(path.to_str().unwrap()).unwrap()
}

fn rec(pairs: &[(&str, CellValue)]) -> BTreeMap<String, CellValue> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[test]
fn create_assigns_id_and_list_round_trips() {
    let store = temp_store("crud");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(180.5))]))
        .unwrap();
    let id = created.get("id").unwrap().to_display_string();
    assert!(!id.is_empty(), "id auto-assigned");
    let listed = store.list(&view, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].get("weight_lbs"), Some(&CellValue::Float(180.5)));
    assert_eq!(store.pending_count().unwrap(), 1, "created row is dirty");
}

#[test]
fn update_overwrites_and_marks_dirty() {
    let store = temp_store("update");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(180.0))]))
        .unwrap();
    let mut edited = created.clone();
    edited.insert("weight_lbs".into(), CellValue::Float(179.0));
    store.update(&view, edited).unwrap();
    let listed = store.list(&view, None).unwrap();
    assert_eq!(listed[0].get("weight_lbs"), Some(&CellValue::Float(179.0)));
}

#[test]
fn delete_of_never_synced_row_removes_outright() {
    let store = temp_store("del-new");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(1.0))]))
        .unwrap();
    store.delete(&view, &created).unwrap();
    assert!(store.list(&view, None).unwrap().is_empty());
    assert_eq!(store.pending_count().unwrap(), 0, "no tombstone for unsynced row");
}

#[test]
fn delete_of_synced_row_leaves_tombstone() {
    let store = temp_store("del-synced");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let created = store
        .create(&view, rec(&[("weight_lbs", CellValue::Float(1.0))]))
        .unwrap();
    let id = created.get("id").unwrap().to_display_string();
    // Simulate a completed sync: base := data, dirty cleared.
    store.mark_synced(&view.name, &id, &created, Some(0)).unwrap();
    assert_eq!(store.pending_count().unwrap(), 0);
    store.delete(&view, &created).unwrap();
    assert!(store.list(&view, None).unwrap().is_empty(), "hidden from list");
    assert_eq!(store.pending_count().unwrap(), 1, "tombstone pending push");
    let rows = store.rows_for_sync("weight").unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].deleted);
}

#[test]
fn list_orders_newest_first() {
    let store = temp_store("order");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    store.create(&view, rec(&[("weight_lbs", CellValue::Float(1.0))])).unwrap();
    store.create(&view, rec(&[("weight_lbs", CellValue::Float(2.0))])).unwrap();
    let listed = store.list(&view, None).unwrap();
    assert_eq!(listed[0].get("weight_lbs"), Some(&CellValue::Float(2.0)));
}
```

- [x] **Step 2: Run** — `cargo test --test store_unit` → FAIL (methods missing).

- [x] **Step 3: Implement** (append to `impl Store` in `src/store/db.rs`)

```rust
    /// List live rows (tombstones excluded), sheet-ordered
    /// (`sort_key ASC` = top of sheet first). Date filter + time
    /// sort applied in Task 4; for now `on_date` must be `None`.
    pub fn list(
        &self,
        view: &crate::ViewSchema,
        on_date: Option<chrono::NaiveDate>,
    ) -> Result<Vec<Record>, StoreError> {
        let _ = on_date; // wired in Task 4
        let mut stmt = self.conn.prepare(
            "SELECT data FROM rows
             WHERE view_name = ?1 AND deleted = 0
             ORDER BY sort_key ASC",
        )?;
        let rows = stmt.query_map([&view.name], |r| r.get::<_, String>(0))?;
        let mut records = Vec::new();
        for data in rows {
            records.push(serde_json::from_str::<Record>(&data?)?);
        }
        Ok(records)
    }

    /// Insert a new dirty row. Auto-assigns a UUID `id` when the view
    /// declares an `id` dimension and the record lacks one (same rule
    /// as `SheetsRepository::create`). Returns the stored record.
    pub fn create(
        &self,
        view: &crate::ViewSchema,
        mut record: Record,
    ) -> Result<Record, StoreError> {
        use crate::value::CellValue;
        if view.dimension_by_name("id").is_some()
            && record.get("id").map_or(true, |v| v.is_empty())
        {
            record.insert(
                "id".into(),
                CellValue::String(uuid::Uuid::new_v4().to_string()),
            );
        }
        let id = record_id(&record)?;
        let sort_key: i64 = self.conn.query_row(
            "SELECT COALESCE(MIN(sort_key), 1) - 1 FROM rows WHERE view_name = ?1",
            [&view.name],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO rows(view_name, id, data, base, dirty, deleted, sort_key, updated_at)
             VALUES (?1, ?2, ?3, NULL, 1, 0, ?4, ?5)",
            rusqlite::params![
                view.name,
                id,
                serde_json::to_string(&record)?,
                sort_key,
                now_rfc3339(),
            ],
        )?;
        Ok(record)
    }

    /// Overwrite an existing row's data and mark it dirty.
    pub fn update(
        &self,
        view: &crate::ViewSchema,
        record: Record,
    ) -> Result<(), StoreError> {
        let id = record_id(&record)?;
        let n = self.conn.execute(
            "UPDATE rows SET data = ?3, dirty = 1, updated_at = ?4
             WHERE view_name = ?1 AND id = ?2 AND deleted = 0",
            rusqlite::params![
                view.name,
                id,
                serde_json::to_string(&record)?,
                now_rfc3339(),
            ],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(id, view.name.clone()));
        }
        Ok(())
    }

    /// Delete: tombstone if the row has ever synced (`base` non-NULL),
    /// otherwise remove outright. No-op if the row doesn't exist
    /// (mirrors the sheets repo's clean delete semantics).
    pub fn delete(
        &self,
        view: &crate::ViewSchema,
        record: &Record,
    ) -> Result<(), StoreError> {
        let id = record_id(record)?;
        self.conn.execute(
            "DELETE FROM rows WHERE view_name = ?1 AND id = ?2 AND base IS NULL",
            rusqlite::params![view.name, id],
        )?;
        self.conn.execute(
            "UPDATE rows SET deleted = 1, updated_at = ?3
             WHERE view_name = ?1 AND id = ?2",
            rusqlite::params![view.name, id, now_rfc3339()],
        )?;
        Ok(())
    }

    /// Rows with un-pushed local changes (dirty edits + tombstones),
    /// across all views. Drives the "unsynced changes" badge.
    pub fn pending_count(&self) -> Result<i64, StoreError> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM rows WHERE dirty = 1 OR deleted = 1",
            [],
            |r| r.get(0),
        )?)
    }

    // ------------------------------------------------ sync support

    /// Every row for a view, tombstones included — the sync engine's
    /// view of local state.
    pub fn rows_for_sync(&self, view_name: &str) -> Result<Vec<LocalRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, data, base, dirty, deleted FROM rows WHERE view_name = ?1",
        )?;
        let rows = stmt.query_map([view_name], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, data, base, dirty, deleted) = row?;
            out.push(LocalRow {
                id,
                data: serde_json::from_str(&data)?,
                base: base.map(|b| serde_json::from_str(&b)).transpose()?,
                dirty,
                deleted,
            });
        }
        Ok(out)
    }

    /// Sync commit for one row: data + base := `record`, dirty
    /// cleared. Upserts, so it also lands rows pulled from the sheet.
    /// `sort_key` `None` keeps the existing key (new local inserts
    /// hold their negative top-of-sheet key until the next pull).
    pub fn mark_synced(
        &self,
        view_name: &str,
        id: &str,
        record: &Record,
        sort_key: Option<i64>,
    ) -> Result<(), StoreError> {
        let data = serde_json::to_string(record)?;
        self.conn.execute(
            "INSERT INTO rows(view_name, id, data, base, dirty, deleted, sort_key, updated_at)
             VALUES (?1, ?2, ?3, ?3, 0, 0, COALESCE(?4, 0), ?5)
             ON CONFLICT(view_name, id) DO UPDATE SET
               data = excluded.data, base = excluded.base,
               dirty = 0, deleted = 0,
               sort_key = COALESCE(?4, rows.sort_key),
               updated_at = excluded.updated_at",
            rusqlite::params![view_name, id, data, sort_key, now_rfc3339()],
        )?;
        Ok(())
    }

    /// Hard-remove a row — remote-deleted rows and pushed tombstones.
    pub fn remove(&self, view_name: &str, id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM rows WHERE view_name = ?1 AND id = ?2",
            rusqlite::params![view_name, id],
        )?;
        Ok(())
    }

    /// Refresh a clean row's sheet-order key after a pull.
    pub fn set_sort_key(
        &self,
        view_name: &str,
        id: &str,
        sort_key: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE rows SET sort_key = ?3 WHERE view_name = ?1 AND id = ?2",
            rusqlite::params![view_name, id, sort_key],
        )?;
        Ok(())
    }
```

Top-level helpers in `db.rs`:

```rust
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn record_id(record: &Record) -> Result<String, StoreError> {
    record
        .get("id")
        .map(|v| v.to_display_string())
        .filter(|s| !s.is_empty())
        .ok_or(StoreError::NoId)
}
```

- [x] **Step 4: Run** — `cargo test --test store_unit` → PASS; `cargo test` all green.

- [x] **Step 5: Commit** — `git commit -am "feat(store): CRUD + tombstones + pending count + sync accessors"`

---

### Task 3: Extract shared date-filter/time-sort into `src/records.rs`

The sheets `list()` date filter + plannable time-of-day sort must behave identically for the local store. Extract, don't duplicate.

**Files:**
- Create: `src/records.rs`
- Modify: `src/sheets/repo.rs` (delegate), `src/lib.rs` (`pub mod records;`)
- Test: `tests/store_unit.rs` (existing sheets tests guard the refactor)

- [x] **Step 1: Create `src/records.rs`**

Move the filtering/sorting tail of `SheetsRepository::list` (repo.rs:239-269) and `parse_time` (repo.rs:567-581) into:

```rust
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
    let Some(on_date) = on_date else { return records };
    let Some(date_field) = view.date_field.clone() else { return records };

    let mut filtered: Vec<Record> = records
        .into_iter()
        .filter(|r| matches!(r.get(&date_field), Some(CellValue::Date(d)) if *d == on_date))
        .collect();

    let Some(log_field) = view.plannable.as_ref().map(|p| p.log_field.clone()) else {
        return filtered;
    };
    filtered.sort_by(|a, b| {
        let av = a.get(&log_field).map(|v| v.to_display_string()).unwrap_or_default();
        let bv = b.get(&log_field).map(|v| v.to_display_string()).unwrap_or_default();
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
        "%-I:%M:%S %p", "%-I:%M %p", "%I:%M:%S %p", "%I:%M %p", "%H:%M:%S", "%H:%M",
    ] {
        if let Ok(t) = NaiveTime::parse_from_str(s, fmt) {
            return Some(t);
        }
    }
    None
}
```

Behavior deltas vs the old inline code, both deliberate: (a) no-`log_field` fallback keeps input order instead of explicitly sorting by `__row` — inputs already arrive in `__row` order, so results are identical; (b) `parse_time` becomes `pub`.

- [x] **Step 2: Rewire `SheetsRepository::list`** — replace repo.rs:239-269 with:

```rust
        Ok(crate::records::filter_and_sort(view, records, on_date))
```

(and change the early-return structure accordingly: build `records`, then always return through `filter_and_sort`; `filter_and_sort` handles the `None` cases). Delete the now-unused `parse_time` and `row_index` helpers from repo.rs.

- [x] **Step 3: Run** — `cargo test` → all existing suites PASS (eval, parse_real_schemas, sheets_unit, sheets_ffi).

- [x] **Step 4: Commit** — `git commit -am "refactor: extract shared date-filter/time-sort into records module"`

---

### Task 4: Store list — date filter + sort via shared helper

**Files:**
- Modify: `src/store/db.rs`
- Test: `tests/store_unit.rs`

- [x] **Step 1: Write failing test** (append)

```rust
#[test]
fn list_filters_by_date_and_sorts_by_log_time() {
    let store = temp_store("datefilter");
    let view = parse_view(
        r#"
name: strength
datasource: gsheets
table: strength
date_field: date
plannable: { log_field: time }
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: date, expr: date }
  - { name: time, type: string, expr: time }
"#,
    )
    .unwrap();
    let d = |s: &str| CellValue::Date(chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap());
    store.create(&view, rec(&[("date", d("2026-08-27")), ("time", CellValue::String("18:00".into()))])).unwrap();
    store.create(&view, rec(&[("date", d("2026-08-27")), ("time", CellValue::String("07:30".into()))])).unwrap();
    store.create(&view, rec(&[("date", d("2026-08-26")), ("time", CellValue::String("09:00".into()))])).unwrap();

    let on = chrono::NaiveDate::parse_from_str("2026-08-27", "%Y-%m-%d").unwrap();
    let listed = store.list(&view, Some(on)).unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].get("time"), Some(&CellValue::String("07:30".into())));
}
```

Note: the test view needs `plannable` to parse — check `parse_view` accepts inline `plannable: { log_field: time }`; if `plannable` only comes from input overlays, build the view with `parse_view_pair`-equivalent (`apply_overlay`) instead. Adjust the fixture, not the store code.

- [x] **Step 2: Run** — FAIL (dates unfiltered).

- [x] **Step 3: Implement** — in `Store::list`, replace the `let _ = on_date;` pass-through:

```rust
        Ok(crate::records::filter_and_sort(view, records, on_date))
```

- [x] **Step 4: Run** — `cargo test --test store_unit` → PASS.

- [x] **Step 5: Commit** — `git commit -am "feat(store): date filter + time sort on list via shared helper"`

---

### Task 5: Pure three-way merge

**Files:**
- Create: `src/sync/mod.rs`, `src/sync/merge.rs`
- Modify: `src/lib.rs` (`pub mod sync;`)
- Test: `tests/sync_merge.rs`

- [x] **Step 1: Write failing table-driven tests**

`tests/sync_merge.rs`:

```rust
use airledger_engine::store::LocalRow;
use airledger_engine::sync::{merge, Action, RemoteRow};
use airledger_engine::value::{CellValue, Record};

fn r(v: f64) -> Record {
    [("id".to_string(), CellValue::String("A".into())),
     ("weight_lbs".to_string(), CellValue::Float(v))]
        .into_iter().collect()
}
fn local(data: Record, base: Option<Record>, dirty: bool, deleted: bool) -> LocalRow {
    LocalRow { id: "A".into(), data, base, dirty, deleted }
}
fn remote(data: Record) -> RemoteRow {
    RemoteRow { id: "A".into(), data, row_index: 0 }
}

#[test]
fn clean_remote_changed_takes_remote() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, false)], &[remote(r(2.0))]);
    assert!(matches!(plan.actions[..], [Action::TakeRemote { .. }]));
    assert_eq!(plan.conflicts, 0);
}

#[test]
fn dirty_remote_unchanged_pushes_local() {
    let plan = merge(&[local(r(2.0), Some(r(1.0)), true, false)], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::PushUpdate { .. }]));
    assert_eq!(plan.conflicts, 0);
}

#[test]
fn dirty_remote_changed_app_wins_and_counts_conflict() {
    let plan = merge(&[local(r(3.0), Some(r(1.0)), true, false)], &[remote(r(2.0))]);
    assert!(matches!(plan.actions[..], [Action::PushUpdate { .. }]));
    assert_eq!(plan.conflicts, 1);
}

#[test]
fn clean_remote_gone_deletes_locally() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, false)], &[]);
    assert!(matches!(plan.actions[..], [Action::DeleteLocal { .. }]));
}

#[test]
fn tombstone_deletes_remote_row() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, true)], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::DeleteRemote { .. }]));
}

#[test]
fn tombstone_with_remote_already_gone_purges_locally() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, true)], &[]);
    assert!(matches!(plan.actions[..], [Action::DeleteLocal { .. }]));
}

#[test]
fn new_local_row_inserts_remotely() {
    let plan = merge(&[local(r(1.0), None, true, false)], &[]);
    assert!(matches!(plan.actions[..], [Action::PushInsert { .. }]));
}

#[test]
fn remote_only_row_is_pulled() {
    let plan = merge(&[], &[remote(r(1.0))]);
    assert!(matches!(plan.actions[..], [Action::TakeRemote { .. }]));
}

#[test]
fn clean_unchanged_yields_no_action() {
    let plan = merge(&[local(r(1.0), Some(r(1.0)), false, false)], &[remote(r(1.0))]);
    assert!(plan.actions.is_empty());
}

#[test]
fn dirty_with_remote_gone_reinserts_app_wins() {
    let plan = merge(&[local(r(2.0), Some(r(1.0)), true, false)], &[]);
    assert!(matches!(plan.actions[..], [Action::PushInsert { .. }]));
    assert_eq!(plan.conflicts, 1);
}
```

- [x] **Step 2: Run** — `cargo test --test sync_merge` → compile error.

- [x] **Step 3: Implement**

`src/sync/mod.rs`:

```rust
//! Sync — bidirectional local-store ⇄ Sheets reconciliation.
//! `merge` is the pure decision core; `engine` (Task 6) executes.

mod merge;

pub use merge::{merge, Action, MergePlan, RemoteRow};
```

`src/sync/merge.rs`:

```rust
use std::collections::BTreeMap;

use crate::store::LocalRow;
use crate::value::Record;

/// One decoded remote row with its zero-based data row index.
#[derive(Debug, Clone)]
pub struct RemoteRow {
    pub id: String,
    pub data: Record,
    pub row_index: usize,
}

/// One reconciliation decision. Remote-touching actions carry the
/// row index from the pull snapshot; the executor must apply
/// `PushUpdate`s before `DeleteRemote`s (descending) before
/// `PushInsert`s so snapshot indexes stay valid.
#[derive(Debug, Clone)]
pub enum Action {
    /// Remote changed, local clean — overwrite local with remote.
    TakeRemote { id: String, data: Record, row_index: usize },
    /// Local dirty — overwrite the sheet row with local data.
    PushUpdate { id: String, row_index: usize },
    /// Locally new (or app-wins resurrection) — insert at sheet top.
    PushInsert { id: String },
    /// Row gone remotely (or tombstone with remote gone) — drop local.
    DeleteLocal { id: String },
    /// Local tombstone — delete the sheet row, then drop local.
    DeleteRemote { id: String, row_index: usize },
}

#[derive(Debug, Default)]
pub struct MergePlan {
    pub actions: Vec<Action>,
    /// Rows where both sides changed since last sync (app won).
    pub conflicts: usize,
}

/// Pure three-way merge: local rows (with `base` = last-synced remote
/// copy) vs the current remote snapshot. No I/O. Spec matrix:
/// docs/superpowers/specs/2026-08-27-local-first-sync-design.md §2.
pub fn merge(local: &[LocalRow], remote: &[RemoteRow]) -> MergePlan {
    let remote_by_id: BTreeMap<&str, &RemoteRow> =
        remote.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut plan = MergePlan::default();
    let mut seen = std::collections::BTreeSet::new();

    for l in local {
        seen.insert(l.id.as_str());
        let r = remote_by_id.get(l.id.as_str()).copied();
        if l.deleted {
            match r {
                Some(r) => plan.actions.push(Action::DeleteRemote {
                    id: l.id.clone(),
                    row_index: r.row_index,
                }),
                None => plan.actions.push(Action::DeleteLocal { id: l.id.clone() }),
            }
            continue;
        }
        match (&l.base, r) {
            // Never synced: push as a new sheet row. (A remote row
            // with the same UUID shouldn't exist; if it does, the
            // insert-at-top still wins visibly and the next pull
            // reconciles — acceptable for UUID collisions.)
            (None, _) => plan.actions.push(Action::PushInsert { id: l.id.clone() }),
            (Some(base), Some(r)) => {
                let remote_changed = r.data != *base;
                match (l.dirty, remote_changed) {
                    (false, false) => {}
                    (false, true) => plan.actions.push(Action::TakeRemote {
                        id: l.id.clone(),
                        data: r.data.clone(),
                        row_index: r.row_index,
                    }),
                    (true, false) => plan.actions.push(Action::PushUpdate {
                        id: l.id.clone(),
                        row_index: r.row_index,
                    }),
                    (true, true) => {
                        plan.conflicts += 1;
                        plan.actions.push(Action::PushUpdate {
                            id: l.id.clone(),
                            row_index: r.row_index,
                        });
                    }
                }
            }
            (Some(_), None) => {
                if l.dirty {
                    // Deleted in the sheet while edited in the app —
                    // app wins: resurrect as a new row.
                    plan.conflicts += 1;
                    plan.actions.push(Action::PushInsert { id: l.id.clone() });
                } else {
                    plan.actions.push(Action::DeleteLocal { id: l.id.clone() });
                }
            }
        }
    }

    for r in remote {
        if !seen.contains(r.id.as_str()) {
            plan.actions.push(Action::TakeRemote {
                id: r.id.clone(),
                data: r.data.clone(),
                row_index: r.row_index,
            });
        }
    }
    plan
}
```

- [x] **Step 4: Run** — `cargo test --test sync_merge` → PASS.

- [x] **Step 5: Commit** — `git commit -am "feat(sync): pure three-way merge with app-wins conflict rule"`

---

### Task 6: Sync engine — `SyncRemote` trait + orchestration

**Files:**
- Create: `src/sync/engine.rs`
- Modify: `src/sync/mod.rs`, `src/sheets/repo.rs` (trait impl)
- Test: `tests/sync_engine.rs` (in-memory `FakeRemote`)

- [x] **Step 1: Write failing tests**

`tests/sync_engine.rs` — a `FakeRemote` holding `RefCell<Vec<Record>>` per view (each record carries `__row` recomputed after mutations, insert-at-front on `push_insert`), then:

```rust
use std::cell::RefCell;
use std::collections::BTreeMap;

use airledger_engine::parse_view;
use airledger_engine::sheets::SheetsError;
use airledger_engine::store::Store;
use airledger_engine::sync::{sync_views, SyncRemote};
use airledger_engine::value::{CellValue, Record};
use airledger_engine::ViewSchema;

const WEIGHT_VIEW: &str = r#"
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: weight_lbs, type: number, expr: weight_lbs }
"#;

/// In-memory stand-in for the sheet: rows newest-first, like the
/// real repo's insert-at-row-2.
#[derive(Default)]
struct FakeRemote {
    rows: RefCell<Vec<Record>>,
    fail_pushes: bool,
}

impl FakeRemote {
    fn with_rows(rows: Vec<Record>) -> Self {
        Self { rows: RefCell::new(rows), fail_pushes: false }
    }
}

impl SyncRemote for FakeRemote {
    fn ensure(&self, _view: &ViewSchema) -> Result<(), SheetsError> { Ok(()) }
    fn pull(&self, _view: &ViewSchema) -> Result<Vec<Record>, SheetsError> {
        Ok(self.rows.borrow().iter().enumerate().map(|(i, r)| {
            let mut r = r.clone();
            r.insert("__row".into(), CellValue::Int(i as i64));
            r
        }).collect())
    }
    fn push_update(&self, _view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        if self.fail_pushes { return Err(SheetsError::Other("boom".into())); }
        let idx = match record.get("__row") { Some(CellValue::Int(i)) => *i as usize, _ => panic!() };
        let mut clean = record.clone();
        clean.remove("__row");
        self.rows.borrow_mut()[idx] = clean;
        Ok(())
    }
    fn push_insert(&self, _view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        if self.fail_pushes { return Err(SheetsError::Other("boom".into())); }
        self.rows.borrow_mut().insert(0, record.clone());
        Ok(())
    }
    fn push_delete(&self, _view: &ViewSchema, row_index: usize) -> Result<(), SheetsError> {
        if self.fail_pushes { return Err(SheetsError::Other("boom".into())); }
        self.rows.borrow_mut().remove(row_index);
        Ok(())
    }
}

fn temp_store(tag: &str) -> Store { /* same helper as store_unit */ }
fn rec(id: &str, v: f64) -> Record {
    [("id".to_string(), CellValue::String(id.into())),
     ("weight_lbs".to_string(), CellValue::Float(v))].into_iter().collect()
}

#[test]
fn initial_sync_hydrates_empty_store() {
    let store = temp_store("hydrate");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0), rec("B", 2.0)]);
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none());
    assert_eq!(results[0].pulled, 2);
    assert_eq!(store.list(&view, None).unwrap().len(), 2);
    assert_eq!(store.pending_count().unwrap(), 0);
}

#[test]
fn local_create_pushes_and_clears_dirty() {
    let store = temp_store("push");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::default();
    store.create(&view, rec("", 3.0)).unwrap(); // id auto-assigned
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert_eq!(results[0].pushed, 1);
    assert_eq!(remote.rows.borrow().len(), 1);
    assert_eq!(store.pending_count().unwrap(), 0);
}

#[test]
fn full_round_trip_bidirectional() {
    let store = temp_store("bidir");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0)]);
    sync_views(&store, &remote, &[view.clone()]);
    // Sheet edit + local edit on different rows.
    remote.rows.borrow_mut()[0] = rec("A", 9.0);
    let created = store.create(&view, rec("", 5.0)).unwrap();
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none());
    let local = store.list(&view, None).unwrap();
    assert_eq!(local.len(), 2);
    assert!(local.iter().any(|r| r.get("weight_lbs") == Some(&CellValue::Float(9.0))), "sheet edit pulled");
    assert_eq!(remote.rows.borrow().len(), 2, "local create pushed");
    let _ = created;
}

#[test]
fn conflict_app_wins() {
    let store = temp_store("conflict");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0)]);
    sync_views(&store, &remote, &[view.clone()]);
    remote.rows.borrow_mut()[0] = rec("A", 100.0); // sheet edit
    let mut edited = store.list(&view, None).unwrap().remove(0);
    edited.insert("weight_lbs".into(), CellValue::Float(50.0));
    store.update(&view, edited).unwrap(); // app edit, same row
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert_eq!(results[0].conflicts, 1);
    assert_eq!(remote.rows.borrow()[0].get("weight_lbs"), Some(&CellValue::Float(50.0)));
}

#[test]
fn tombstone_deletes_remote_and_purges() {
    let store = temp_store("tomb");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let remote = FakeRemote::with_rows(vec![rec("A", 1.0)]);
    sync_views(&store, &remote, &[view.clone()]);
    let row = store.list(&view, None).unwrap().remove(0);
    store.delete(&view, &row).unwrap();
    sync_views(&store, &remote, &[view.clone()]);
    assert!(remote.rows.borrow().is_empty());
    assert_eq!(store.rows_for_sync("weight").unwrap().len(), 0, "tombstone purged");
}

#[test]
fn idless_remote_rows_get_ids_written_back() {
    let store = temp_store("idless");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let mut no_id = Record::new();
    no_id.insert("weight_lbs".into(), CellValue::Float(7.0));
    let remote = FakeRemote::with_rows(vec![no_id]);
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none());
    let sheet_id = remote.rows.borrow()[0].get("id").unwrap().to_display_string();
    assert!(!sheet_id.is_empty(), "id written back to sheet");
    let local = store.list(&view, None).unwrap();
    assert_eq!(local[0].get("id").unwrap().to_display_string(), sheet_id);
}

#[test]
fn push_failure_keeps_dirty_for_retry() {
    let store = temp_store("retry");
    let view = parse_view(WEIGHT_VIEW).unwrap();
    let mut remote = FakeRemote::default();
    remote.fail_pushes = true;
    store.create(&view, rec("", 1.0)).unwrap();
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_some());
    assert_eq!(store.pending_count().unwrap(), 1, "still dirty");
    // Remote heals → retry succeeds.
    remote.fail_pushes = false;
    let results = sync_views(&store, &remote, &[view.clone()]);
    assert!(results[0].error.is_none());
    assert_eq!(store.pending_count().unwrap(), 0);
}
```

- [x] **Step 2: Run** — compile error (no `sync_views` / `SyncRemote`).

- [x] **Step 3: Implement**

`src/sync/engine.rs`:

```rust
use serde::Serialize;

use crate::schema::view::ViewSchema;
use crate::sheets::{SheetsError, SheetsRepository, ROW_INDEX_KEY};
use crate::store::Store;
use crate::value::{CellValue, Record};

use super::merge::{merge, Action, RemoteRow};

/// The remote side of a sync, abstracted so the engine is testable
/// without network. `pull` returns decoded records carrying
/// `__row`; push ops mirror `SheetsRepository`'s addressing.
pub trait SyncRemote {
    fn ensure(&self, view: &ViewSchema) -> Result<(), SheetsError>;
    fn pull(&self, view: &ViewSchema) -> Result<Vec<Record>, SheetsError>;
    /// Overwrite the row at `record["__row"]` with `record`.
    fn push_update(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError>;
    /// Insert `record` at the top of the sheet (data row 0).
    fn push_insert(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError>;
    fn push_delete(&self, view: &ViewSchema, row_index: usize) -> Result<(), SheetsError>;
}

impl SyncRemote for SheetsRepository {
    fn ensure(&self, view: &ViewSchema) -> Result<(), SheetsError> {
        self.ensure_sheet(view)
    }
    fn pull(&self, view: &ViewSchema) -> Result<Vec<Record>, SheetsError> {
        self.list(view, None)
    }
    fn push_update(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        self.update(view, record.clone())
    }
    fn push_insert(&self, view: &ViewSchema, record: &Record) -> Result<(), SheetsError> {
        let mut r = record.clone();
        r.remove(ROW_INDEX_KEY);
        self.create(view, r).map(|_| ())
    }
    fn push_delete(&self, view: &ViewSchema, row_index: usize) -> Result<(), SheetsError> {
        let mut r = Record::new();
        r.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(row_index as i64));
        self.delete(view, &r)
    }
}

/// Per-view sync outcome — serialized over FFI as the sync summary.
#[derive(Debug, Serialize)]
pub struct ViewSyncResult {
    pub view: String,
    pub pulled: usize,
    pub pushed: usize,
    pub deleted_local: usize,
    pub deleted_remote: usize,
    pub conflicts: usize,
    pub error: Option<String>,
}

/// Sync every view: pull → merge → push → commit, per the spec.
/// Commit is per-action (dirty clears only after that row's push
/// succeeds), so an abort anywhere leaves a retryable state.
/// A view's error aborts that view only; later views still sync.
pub fn sync_views(
    store: &Store,
    remote: &dyn SyncRemote,
    views: &[ViewSchema],
) -> Vec<ViewSyncResult> {
    views.iter().map(|v| sync_one(store, remote, v)).collect()
}

fn sync_one(store: &Store, remote: &dyn SyncRemote, view: &ViewSchema) -> ViewSyncResult {
    let mut res = ViewSyncResult {
        view: view.name.clone(),
        pulled: 0, pushed: 0, deleted_local: 0, deleted_remote: 0,
        conflicts: 0, error: None,
    };
    if let Err(e) = sync_one_inner(store, remote, view, &mut res) {
        res.error = Some(e);
    }
    res
}

fn sync_one_inner(
    store: &Store,
    remote: &dyn SyncRemote,
    view: &ViewSchema,
    res: &mut ViewSyncResult,
) -> Result<(), String> {
    remote.ensure(view).map_err(|e| format!("ensure: {e}"))?;

    // Pull + normalize: strip __row into row_index, assign ids to
    // id-less rows (written back immediately so the sheet row is
    // addressable), dedup ids (first wins).
    let pulled = remote.pull(view).map_err(|e| format!("pull: {e}"))?;
    let mut remote_rows: Vec<RemoteRow> = Vec::new();
    let mut seen_ids = std::collections::BTreeSet::new();
    for mut rec in pulled {
        let row_index = match rec.remove(ROW_INDEX_KEY) {
            Some(CellValue::Int(i)) => i as usize,
            _ => continue,
        };
        let id = rec.get("id").map(|v| v.to_display_string()).unwrap_or_default();
        let id = if id.is_empty() {
            let new_id = uuid::Uuid::new_v4().to_string();
            rec.insert("id".into(), CellValue::String(new_id.clone()));
            let mut with_row = rec.clone();
            with_row.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(row_index as i64));
            remote
                .push_update(view, &with_row)
                .map_err(|e| format!("id write-back: {e}"))?;
            new_id
        } else {
            id
        };
        if !seen_ids.insert(id.clone()) {
            continue; // duplicate id in sheet — first wins
        }
        remote_rows.push(RemoteRow { id, data: rec, row_index });
    }

    let local = store
        .rows_for_sync(&view.name)
        .map_err(|e| format!("local read: {e}"))?;
    let local_by_id: std::collections::BTreeMap<&str, &crate::store::LocalRow> =
        local.iter().map(|l| (l.id.as_str(), l)).collect();
    let plan = merge(&local, &remote_rows);
    res.conflicts = plan.conflicts;

    // Ordering contract (see merge::Action docs): updates while the
    // snapshot indexes are valid, then deletes bottom-up, then
    // inserts. Local-only actions can run anytime; they run in
    // their phase's pass.
    let mut updates = Vec::new();
    let mut deletes = Vec::new();
    let mut inserts = Vec::new();
    for a in &plan.actions {
        match a {
            Action::TakeRemote { id, data, row_index } => {
                store
                    .mark_synced(&view.name, id, data, Some(*row_index as i64))
                    .map_err(|e| format!("take remote: {e}"))?;
                res.pulled += 1;
            }
            Action::DeleteLocal { id } => {
                store.remove(&view.name, id).map_err(|e| format!("local delete: {e}"))?;
                res.deleted_local += 1;
            }
            Action::PushUpdate { .. } => updates.push(a.clone()),
            Action::DeleteRemote { .. } => deletes.push(a.clone()),
            Action::PushInsert { .. } => inserts.push(a.clone()),
        }
    }

    for a in updates {
        let Action::PushUpdate { id, row_index } = a else { unreachable!() };
        let l = local_by_id.get(id.as_str()).expect("merge only names local ids");
        let mut rec = l.data.clone();
        rec.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(row_index as i64));
        remote.push_update(view, &rec).map_err(|e| format!("push update: {e}"))?;
        store
            .mark_synced(&view.name, &id, &l.data, Some(row_index as i64))
            .map_err(|e| format!("commit update: {e}"))?;
        res.pushed += 1;
    }

    deletes.sort_by_key(|a| {
        let Action::DeleteRemote { row_index, .. } = a else { unreachable!() };
        std::cmp::Reverse(*row_index)
    });
    for a in deletes {
        let Action::DeleteRemote { id, row_index } = a else { unreachable!() };
        remote.push_delete(view, row_index).map_err(|e| format!("push delete: {e}"))?;
        store.remove(&view.name, &id).map_err(|e| format!("commit delete: {e}"))?;
        res.deleted_remote += 1;
    }

    for a in inserts {
        let Action::PushInsert { id } = a else { unreachable!() };
        let l = local_by_id.get(id.as_str()).expect("merge only names local ids");
        remote.push_insert(view, &l.data).map_err(|e| format!("push insert: {e}"))?;
        store
            .mark_synced(&view.name, &id, &l.data, None)
            .map_err(|e| format!("commit insert: {e}"))?;
        res.pushed += 1;
    }

    // Refresh sheet-order keys for rows untouched this round.
    for r in &remote_rows {
        store
            .set_sort_key(&view.name, &r.id, r.row_index as i64)
            .map_err(|e| format!("sort key: {e}"))?;
    }
    store
        .meta_set(&format!("last_sync_{}", view.name), &chrono::Utc::now().to_rfc3339())
        .map_err(|e| format!("meta: {e}"))?;
    Ok(())
}
```

Update `src/sync/mod.rs`:

```rust
mod engine;
mod merge;

pub use engine::{sync_views, SyncRemote, ViewSyncResult};
pub use merge::{merge, Action, MergePlan, RemoteRow};
```

Caveat discovered during test-writing may apply: `store.mark_synced` for `TakeRemote` runs before pushes; if a later push fails, pulled rows are already committed — that's correct (pull results are valid regardless of push outcome).

- [x] **Step 4: Run** — `cargo test --test sync_engine` and full `cargo test` → PASS.

- [x] **Step 5: Commit** — `git commit -am "feat(sync): sync engine — SyncRemote trait, pull/merge/push/commit"`

---

### Task 7: FFI ledger surface

**Files:**
- Modify: `src/ffi.rs`
- Test: `tests/ledger_ffi.rs` (patterned on `tests/sheets_ffi.rs`)

- [x] **Step 1: Write failing test** — `tests/ledger_ffi.rs` exercising via the C ABI: open a ledger with a temp db path + the fake SA json from `tests/sheets_ffi.rs` (PEM is validated lazily at token time, so offline CRUD works), then create → list → pending round-trip through `airledger_engine_ledger_*` symbols, asserting the JSON envelopes. Follow `tests/sheets_ffi.rs` for CString marshaling helpers.

- [x] **Step 2: Run** — FAIL (symbols missing).

- [x] **Step 3: Implement** — append to `src/ffi.rs`:

```rust
// ========================================================== ledger handle
//
// Local-first store + sync. Mirrors the sheets handle pattern: the
// store and the sheets repo live behind one opaque handle; local CRUD
// never touches the network, sync does.

use crate::store::Store;
use crate::sync::sync_views;

pub struct LedgerHandle {
    store: Mutex<Store>,
    sheets: Mutex<SheetsRepository>,
}

/// Open the local store at `db_path` and prepare the sheets repo for
/// sync. No network here — credentials are parsed, not exercised.
/// Returns NULL on failure with the message in `error_out`.
///
/// # Safety
/// String pointers must be valid nul-terminated UTF-8. `error_out`
/// must be NULL or a valid writable `*mut *mut c_char`.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_open(
    db_path_ptr: *const c_char,
    default_spreadsheet_id_ptr: *const c_char,
    service_account_json_ptr: *const c_char,
    error_out: *mut *mut c_char,
) -> *mut LedgerHandle {
    let result = (|| -> Result<LedgerHandle, String> {
        let db_path = unsafe { c_str_to_str(db_path_ptr) }?;
        let sid = unsafe { c_str_to_str(default_spreadsheet_id_ptr) }?;
        let sa = unsafe { c_str_to_str(service_account_json_ptr) }?;
        let store = Store::open(db_path).map_err(|e| e.to_string())?;
        let sheets =
            SheetsRepository::new(sid.to_string(), sa).map_err(|e| e.to_string())?;
        Ok(LedgerHandle { store: Mutex::new(store), sheets: Mutex::new(sheets) })
    })();
    match result {
        Ok(h) => {
            if !error_out.is_null() {
                unsafe { *error_out = std::ptr::null_mut() };
            }
            Box::into_raw(Box::new(h))
        }
        Err(e) => {
            if !error_out.is_null() {
                unsafe { *error_out = string_to_ptr(e) };
            }
            std::ptr::null_mut()
        }
    }
}

/// # Safety
/// At most once per handle; handle unusable afterwards.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_free_handle(handle: *mut LedgerHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}
```

Then `ledger_list` / `ledger_create` / `ledger_update` / `ledger_delete` with the exact same shapes as their `sheets_` counterparts (same view-JSON + record-JSON in, same envelopes out), routed through a `ledger_call` helper that locks `handle.store` and maps `StoreError` via `e.to_string()`; `ledger_list` parses the optional `YYYY-MM-DD` third argument exactly as `airledger_engine_sheets_list` does. Plus:

```rust
/// Pending (un-pushed) change count: `{"pending": N}`.
///
/// # Safety
/// `handle` must be a valid ledger handle.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_pending(
    handle: *mut LedgerHandle,
) -> *mut c_char {
    if handle.is_null() {
        return error_json("null handle");
    }
    let handle = unsafe { &*handle };
    let store = match handle.store.lock() {
        Ok(g) => g,
        Err(_) => return error_json("store mutex poisoned"),
    };
    match store.pending_count() {
        Ok(n) => result_json(&serde_json::json!({ "pending": n })),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Run a full sync for every view in `views_json_ptr` (a JSON array
/// of ViewSchema). Returns the JSON array of per-view results —
/// individual view failures land in each result's `error` field, so
/// this call only returns `{"error": ...}` for input-shape problems.
///
/// # Safety
/// `handle` valid; `views_json_ptr` nul-terminated UTF-8.
#[no_mangle]
pub unsafe extern "C" fn airledger_engine_ledger_sync(
    handle: *mut LedgerHandle,
    views_json_ptr: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return error_json("null handle");
    }
    let views_json = match unsafe { c_str_to_str(views_json_ptr) } {
        Ok(s) => s,
        Err(e) => return error_json(&e),
    };
    let views: Vec<ViewSchema> = match serde_json::from_str(views_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("views json: {e}")),
    };
    let handle = unsafe { &*handle };
    let (store, sheets) = match (handle.store.lock(), handle.sheets.lock()) {
        (Ok(s), Ok(sh)) => (s, sh),
        _ => return error_json("handle mutex poisoned"),
    };
    result_json(&sync_views(&store, &*sheets, &views))
}
```

- [x] **Step 4: Run** — `cargo test --test ledger_ffi` and `cargo test` → PASS.

- [x] **Step 5: Commit** — `git commit -am "feat(ffi): ledger handle — local CRUD, pending, sync"`

---

### Task 8: sdk-dart — bindings + `EngineLedgerRepository`

**Files:**
- Modify: `sdk-dart/lib/src/bindings.dart`, `sdk-dart/lib/src/airledger_engine_base.dart`, `sdk-dart/lib/airledger_engine.dart` (exports, if it lists symbols)
- Test: `sdk-dart/test/ledger_test.dart`

- [x] **Step 1: Bindings** — in `bindings.dart` add `typedef LedgerHandle = ffi.Void;` plus lookups mirroring the sheets set: `ledgerOpen` (4-arg: three strings + error-out, returns `Pointer<LedgerHandle>`), `ledgerFreeHandle`, `ledgerList` (handle + view + nullable date), `ledgerCreate`/`ledgerUpdate`/`ledgerDelete` (handle + view + record), `ledgerPending` (handle only), `ledgerSync` (handle + views-array json). Same typedef style as the sheets entries.

- [x] **Step 2: Failing Dart test** — `sdk-dart/test/ledger_test.dart`:

```dart
import 'dart:io';
import 'package:airledger_engine/airledger_engine.dart';
import 'package:test/test.dart';

const fakeSa = '''
{"type":"service_account","client_email":"t@example.iam.gserviceaccount.com",
 "private_key":"-----BEGIN PRIVATE KEY-----\\nZmFrZQ==\\n-----END PRIVATE KEY-----\\n",
 "private_key_id":"k1","token_uri":"https://oauth2.googleapis.com/token"}
''';

const viewYaml = '''
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: weight_lbs, type: number, expr: weight_lbs }
''';

void main() {
  test('ledger CRUD works offline and tracks pending', () async {
    final engine = AirledgerEngine.load();
    final dir = Directory.systemTemp.createTempSync('ledger');
    final ledger = engine.openLedger(
      dbPath: '${dir.path}/t.db',
      defaultSpreadsheetId: 'unused',
      serviceAccountJson: fakeSa,
    );
    final view = engine.parseView(viewYaml);
    final created = await ledger.create(view, recordToEngineJson({'weight_lbs': 180.5}));
    expect(created['id'], isNotNull);
    final rows = await ledger.list(view);
    expect(rows, hasLength(1));
    expect(await ledger.pending(), 1);
    await ledger.delete(view, created);
    expect(await ledger.list(view), isEmpty);
    expect(await ledger.pending(), 0);
    ledger.close();
    dir.deleteSync(recursive: true);
  });
}
```

(Match the actual return shape of `create` — the FFI returns the stored record envelope, so `created['id']` is a tagged map `{'kind':'string','value':...}`; assert accordingly.)

- [x] **Step 3: Implement wrapper** — in `airledger_engine_base.dart`, add `extension AirledgerEngineLedger on AirledgerEngine { EngineLedgerRepository openLedger({required String dbPath, required String defaultSpreadsheetId, required String serviceAccountJson}) }` following `connectSheets`'s error-out pattern, and `class EngineLedgerRepository` mirroring `EngineSheetsRepository`: Finalizer + `close()`, `list/create/update/delete` via `Isolate.run` worker helpers (own `_LedgerOpKind` enum + `_ledgerRunTwo`/`_ledgerRunList` top-level functions, copy-adapted from the sheets ones), plus:

```dart
  /// Run a full sync. Returns per-view result maps
  /// (`{view, pulled, pushed, deleted_local, deleted_remote,
  /// conflicts, error}`). Network happens here and only here.
  Future<List<Map<String, dynamic>>> sync(List<Map<String, dynamic>> views) async { ... }

  /// Count of local changes not yet pushed.
  Future<int> pending() async { ... }
```

- [x] **Step 4: Run** — `cd sdk-dart && dart test` → all PASS (rebuilds dylib via existing test setup; if not, `cargo build` first).

- [x] **Step 5: Commit** — `git commit -am "feat(sdk-dart): EngineLedgerRepository — offline CRUD, pending, sync"`

---

### Task 9: Env-gated live sync integration test

**Files:**
- Create: `tests/sync_integration.rs` (pattern: `tests/sheets_integration.rs`)

- [x] **Step 1: Write the test** — gated on `AIRLEDGER_SHEETS_TEST_CREDS_PATH` + `AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID` (skip silently when unset, same as sheets_integration). Flow: unique tab name per run → local create → `sync_views` push → mutate the sheet directly via `SheetsRepository::update` (simulating a manual edit) → second store (fresh temp db) hydrates and sees it → local edit + remote edit on same row → sync → assert app-wins on the sheet → local delete → sync → sheet row gone → clean up tab.

- [x] **Step 2: Run** — without env vars: `cargo test --test sync_integration` → skip-pass. With creds (run if `~/.config` or env provides them; otherwise note as deferred): full PASS.

- [x] **Step 3: Commit** — `git commit -am "test(sync): env-gated live round-trip against a real workbook"`

---

### Task 10: Docs + follow-up notes

**Files:**
- Modify: `README.md` (status list), `docs/port-plan.md` (phase table + "what lives where"), spec (follow-ups section)

- [x] **Step 1:** README status: add "✅ Phase 8 — local-first store + sync engine (`store/`, `sync/`, ledger FFI)". Port-plan: add phase 8 row; document the ledger handle in "what lives where"; add a **follow-up section for the archive app wiring** (not implementable here — repo absent): swap `EngineSheetsRepository` → `EngineLedgerRepository` behind `WarehouseConnector`, add `SyncScheduler` (connectivity_plus; triggers: app start/resume, 5 s post-write debounce, wifi-appears-with-pending), `Sync on Wi-Fi only` setting (default on), pending badge + last-synced stamp, and reconcile the `body_fat_withing` column name in `airledger-fitness/views/weight.view.yml` against the actual sheet column.

- [x] **Step 2:** `cargo test` + `cd sdk-dart && dart test` one final time → all green.

- [x] **Step 3: Commit** — `git commit -am "docs: phase 8 status + archive-app sync wiring follow-ups"`

---

## Self-review checklist (done at plan time)

- Spec coverage: §1 data model → Tasks 1–4; §2 sync → Tasks 5–6; §3 FFI → Tasks 7–8; §3 app wiring → Task 10 follow-up (repo absent); §4 error handling → Tasks 2/6 tests; §5 testing → Tasks 2–9.
- Type consistency: `LocalRow`/`RemoteRow`/`Action`/`sync_views` names match across Tasks 2, 5, 6, 7.
- Known judgment calls for the executor: exact rusqlite version (use latest compatible), `plannable` inline-parse in Task 4's fixture (adjust fixture if it only parses via overlay), Dart record-envelope assertions in Task 8.
