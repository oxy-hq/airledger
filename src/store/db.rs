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
}
