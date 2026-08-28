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
