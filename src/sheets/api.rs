//! Thin wrappers over the Sheets v4 REST endpoints we touch.
//!
//! Each method builds a request, sends it with the caller's bearer
//! token, and parses the response. Retry / token-refresh policy lives
//! in [`super::repo`] — these wrappers are one-shot.

use serde::Deserialize;
use serde_json::{json, Value};

use super::SheetsError;

const BASE: &str = "https://sheets.googleapis.com/v4/spreadsheets";

#[derive(Debug, Deserialize)]
pub struct SheetMeta {
    #[serde(rename = "sheetId")]
    pub sheet_id: i64,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct SheetWrap {
    pub properties: SheetMeta,
}

#[derive(Debug, Deserialize)]
pub struct SpreadsheetMeta {
    #[serde(default)]
    pub sheets: Vec<SheetWrap>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ValueRange {
    #[serde(default)]
    pub values: Vec<Vec<Value>>,
}

pub struct Api<'a> {
    pub http: &'a reqwest::blocking::Client,
    pub token: &'a str,
}

impl<'a> Api<'a> {
    /// GET /spreadsheets/{id}
    pub fn get_spreadsheet(
        &self,
        spreadsheet_id: &str,
    ) -> Result<SpreadsheetMeta, SheetsError> {
        let url = format!("{BASE}/{}", urlencoding::encode(spreadsheet_id));
        let resp = self.http.get(&url).bearer_auth(self.token).send()?;
        parse_body(resp)
    }

    /// GET /spreadsheets/{id}/values/{range}
    pub fn get_values(
        &self,
        spreadsheet_id: &str,
        range: &str,
    ) -> Result<ValueRange, SheetsError> {
        let url = format!(
            "{BASE}/{}/values/{}",
            urlencoding::encode(spreadsheet_id),
            urlencoding::encode(range),
        );
        let resp = self.http.get(&url).bearer_auth(self.token).send()?;
        parse_body(resp)
    }

    /// PUT /spreadsheets/{id}/values/{range}?valueInputOption=RAW
    pub fn update_values(
        &self,
        spreadsheet_id: &str,
        range: &str,
        row: Vec<Value>,
    ) -> Result<(), SheetsError> {
        let url = format!(
            "{BASE}/{}/values/{}?valueInputOption=RAW",
            urlencoding::encode(spreadsheet_id),
            urlencoding::encode(range),
        );
        let body = json!({ "values": [row] });
        let resp = self
            .http
            .put(&url)
            .bearer_auth(self.token)
            .json(&body)
            .send()?;
        check_ok(resp)
    }

    /// POST /spreadsheets/{id}:batchUpdate
    pub fn batch_update(
        &self,
        spreadsheet_id: &str,
        requests: Vec<Value>,
    ) -> Result<(), SheetsError> {
        let url = format!("{BASE}/{}:batchUpdate", urlencoding::encode(spreadsheet_id));
        let body = json!({ "requests": requests });
        let resp = self
            .http
            .post(&url)
            .bearer_auth(self.token)
            .json(&body)
            .send()?;
        check_ok(resp)
    }
}

fn parse_body<T: serde::de::DeserializeOwned>(
    resp: reqwest::blocking::Response,
) -> Result<T, SheetsError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(SheetsError::Api {
            status: status.as_u16(),
            body,
        });
    }
    resp.json().map_err(SheetsError::Http)
}

fn check_ok(resp: reqwest::blocking::Response) -> Result<(), SheetsError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(SheetsError::Api {
            status: status.as_u16(),
            body,
        });
    }
    Ok(())
}
