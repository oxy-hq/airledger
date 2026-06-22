//! Service-account JWT auth for the Google Sheets API.
//!
//! Self-signed RS256 JWT → token exchange against `token_uri` →
//! short-lived bearer token. Mirrors the `googleapis_auth` Dart side.
//! Token caching lives one level up in [`super::repo::SheetsRepository`].

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{encode as jwt_encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use super::SheetsError;

const SCOPE: &str = "https://www.googleapis.com/auth/spreadsheets";

/// Parsed service-account JSON. Holds only the fields needed to mint
/// access tokens — everything else from the key file is ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct ServiceAccount {
    pub client_email: String,
    pub private_key: String,
    pub token_uri: String,
    #[serde(default)]
    pub private_key_id: String,
}

impl ServiceAccount {
    pub fn from_json(json: &str) -> Result<Self, SheetsError> {
        serde_json::from_str(json).map_err(SheetsError::ServiceAccountJson)
    }
}

/// A live OAuth2 access token + the wall-clock time it stops being
/// usable.
///
/// `SystemTime` (not `Instant`) on purpose: `Instant` pauses while
/// the device is in deep sleep on Android, so a phone that sleeps
/// for >1h with a cached token would wake up thinking the token is
/// still fresh — but Google's clock kept ticking, and the token has
/// actually expired. Wall-clock advance keeps the cache honest at
/// the cost of being sensitive to local clock jumps (which is fine
/// here — the token's lifetime is a wall-clock thing on Google's
/// side anyway).
#[derive(Debug, Clone)]
pub struct AccessToken {
    pub token: String,
    pub expires_at: SystemTime,
}

impl AccessToken {
    /// Treat the token as expired 60s before the actual deadline so an
    /// in-flight request never races the refresh.
    pub fn is_fresh(&self) -> bool {
        self.expires_at > SystemTime::now() + Duration::from_secs(60)
    }
}

#[derive(Debug, Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: u64,
    iat: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

/// Sign a JWT for `sa` and POST it to its `token_uri` to exchange for
/// an access token.
pub fn fetch_access_token(
    sa: &ServiceAccount,
    http: &reqwest::blocking::Client,
) -> Result<AccessToken, SheetsError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SheetsError::Other("system clock before unix epoch".into()))?
        .as_secs();
    let claims = JwtClaims {
        iss: &sa.client_email,
        scope: SCOPE,
        aud: &sa.token_uri,
        exp: now + 3600,
        iat: now,
    };

    let mut header = Header::new(Algorithm::RS256);
    if !sa.private_key_id.is_empty() {
        header.kid = Some(sa.private_key_id.clone());
    }

    let key = EncodingKey::from_rsa_pem(sa.private_key.as_bytes())
        .map_err(|e| SheetsError::Jwt(format!("private key parse: {e}")))?;
    let assertion = jwt_encode(&header, &claims, &key)
        .map_err(|e| SheetsError::Jwt(format!("sign: {e}")))?;

    let resp = http
        .post(&sa.token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &assertion),
        ])
        .send()
        .map_err(SheetsError::from)?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(SheetsError::TokenExchange(format!("{status}: {body}")));
    }

    let body: TokenResponse = resp.json().map_err(SheetsError::from)?;
    Ok(AccessToken {
        token: body.access_token,
        expires_at: SystemTime::now() + Duration::from_secs(body.expires_in),
    })
}

