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

    /// PUT /spreadsheets/{id}/values/{range}?valueInputOption=USER_ENTERED
    pub fn update_values(
        &self,
        spreadsheet_id: &str,
        range: &str,
        row: Vec<Value>,
    ) -> Result<(), SheetsError> {
        // USER_ENTERED makes Sheets parse incoming strings like a
        // user typed them — "2026-06-20" becomes a real date, "8:55:00
        // PM" a real time, "180" a real number. With RAW everything
        // stays as text, which is why date/time cells used to display
        // with a leading apostrophe in the UI.
        //
        // COROLLARY (learned from the 2026-09-21 429 storm): whatever
        // string forms we write here MUST decode back after Sheets
        // re-renders them — e.g. ISO-T datetimes come back
        // space-separated. See `eval::codec::parse_datetime`.
        let url = format!(
            "{BASE}/{}/values/{}?valueInputOption=USER_ENTERED",
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

    /// GET /spreadsheets/{id}/values:batchGet — many ranges, ONE read
    /// request against the per-minute quota. Ranges come back in
    /// request order.
    pub fn batch_get_values(
        &self,
        spreadsheet_id: &str,
        ranges: &[String],
    ) -> Result<Vec<ValueRange>, SheetsError> {
        #[derive(Deserialize)]
        struct BatchGetResponse {
            #[serde(default, rename = "valueRanges")]
            value_ranges: Vec<ValueRange>,
        }
        let mut url = format!(
            "{BASE}/{}/values:batchGet?majorDimension=ROWS",
            urlencoding::encode(spreadsheet_id),
        );
        for r in ranges {
            url.push_str("&ranges=");
            url.push_str(&urlencoding::encode(r));
        }
        let resp = self.http.get(&url).bearer_auth(self.token).send()?;
        let parsed: BatchGetResponse = parse_body(resp)?;
        Ok(parsed.value_ranges)
    }

    /// POST /spreadsheets/{id}/values:batchUpdate — many single-row
    /// range writes, ONE write request against the per-minute quota.
    /// Same USER_ENTERED semantics as [`Self::update_values`].
    pub fn batch_update_values(
        &self,
        spreadsheet_id: &str,
        data: &[(String, Vec<Value>)],
    ) -> Result<(), SheetsError> {
        let url = format!(
            "{BASE}/{}/values:batchUpdate",
            urlencoding::encode(spreadsheet_id),
        );
        let body = json!({
            "valueInputOption": "USER_ENTERED",
            "data": data
                .iter()
                .map(|(range, row)| json!({ "range": range, "values": [row] }))
                .collect::<Vec<_>>(),
        });
        let resp = self
            .http
            .post(&url)
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
        return Err(api_error(resp));
    }
    resp.json().map_err(SheetsError::from)
}

fn check_ok(resp: reqwest::blocking::Response) -> Result<(), SheetsError> {
    let status = resp.status();
    if !status.is_success() {
        return Err(api_error(resp));
    }
    Ok(())
}

fn api_error(resp: reqwest::blocking::Response) -> SheetsError {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    let body = resp.text().unwrap_or_default();
    SheetsError::Api { status, body, retry_after }
}
