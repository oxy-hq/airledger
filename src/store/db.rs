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
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| {
                r.get(0)
            })
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

    /// List live rows (tombstones excluded), sheet-ordered
    /// (`sort_key ASC` = top of sheet first). Date filter + time
    /// sort match `SheetsRepository::list` via the shared helper.
    pub fn list(
        &self,
        view: &crate::ViewSchema,
        on_date: Option<chrono::NaiveDate>,
    ) -> Result<Vec<Record>, StoreError> {
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
        Ok(crate::records::filter_and_sort(view, records, on_date))
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
}

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
