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

use std::error::Error as StdError;

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
    // Wrap reqwest errors with the full source chain — the default
    // Display on reqwest::Error is "error sending request for url X"
    // with no underlying cause, which makes Android TLS / DNS issues
    // impossible to diagnose without this.
    #[error("http: {0}")]
    Http(#[from] HttpError),
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

/// Wrapper around `reqwest::Error` that prints its full source chain.
/// reqwest's own `Display` impl stops at the first level, which makes
/// it useless for diagnosing TLS / DNS / certificate failures.
#[derive(Debug)]
pub struct HttpError(pub reqwest::Error);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(e) = source {
            write!(f, ": {e}")?;
            source = e.source();
        }
        Ok(())
    }
}

impl StdError for HttpError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.0.source()
    }
}

impl From<reqwest::Error> for HttpError {
    fn from(e: reqwest::Error) -> Self {
        HttpError(e)
    }
}

// `?` needs a one-step From; provide it so call sites can keep using
// `?` on reqwest results without an explicit `.map_err`.
impl From<reqwest::Error> for SheetsError {
    fn from(e: reqwest::Error) -> Self {
        SheetsError::Http(HttpError(e))
    }
}

impl HttpError {
    pub fn is_connect(&self) -> bool {
        self.0.is_connect()
    }
    pub fn is_timeout(&self) -> bool {
        self.0.is_timeout()
    }
}
