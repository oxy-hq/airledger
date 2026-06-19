//! Sheets ingest — port of
//! `airledger/lib/services/sheets_repository.dart`.
//!
//! The Dart code used `googleapis_auth` + `googleapis/sheets/v4.dart`.
//! Here we talk to the REST endpoints directly via `reqwest::blocking`,
//! mint our own RS256 JWT, and exchange it for an access token.
//!
//! Synchronous on purpose — the FFI surface is sync, and Dart consumers
//! call into Rust from a worker isolate. WASM (Phase 5) will need its
//! own async fetch-based binding; that's a separate module.

mod api;
mod auth;
mod repo;

use thiserror::Error;

pub use auth::ServiceAccount;
pub use repo::{shift_row_indexes, SheetsRepository, ROW_INDEX_KEY};

/// All errors the sheets module produces.
#[derive(Error, Debug)]
pub enum SheetsError {
    #[error("service account json: {0}")]
    ServiceAccountJson(serde_json::Error),
    #[error("jwt: {0}")]
    Jwt(String),
    #[error("token exchange failed: {0}")]
    TokenExchange(String),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("sheets api {status}: {body}")]
    Api { status: u16, body: String },
    #[error("cannot resolve row: no __row index and no id on record")]
    NoRowRef,
    #[error("no sheet tab named \"{0}\"")]
    MissingTab(String),
    #[error("no row with id=\"{0}\" in \"{1}\"")]
    IdNotFound(String, String),
    #[error("{0}")]
    Other(String),
}
