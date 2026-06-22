//! `SheetsRepository` — the CRUD surface mirroring
//! `SheetsRepository` in `airledger/lib/services/sheets_repository.dart`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use chrono::{NaiveDate, NaiveTime};
use serde_json::Value;
use uuid::Uuid;

use crate::eval::codec::{decode, encode};
use crate::schema::view::ViewSchema;
use crate::value::{CellValue, Record};

use super::api::Api;
use super::auth::{fetch_access_token, AccessToken, ServiceAccount};
use super::SheetsError;

const RETRY_ATTEMPTS: u32 = 4;
const RETRY_BASE_MS: u64 = 300;

/// Key set on records loaded from the sheet, carrying their zero-based
/// data row index so `update` / `delete` can find them without an id.
/// Mirrors `rowIndexKey` on the Dart side.
pub const ROW_INDEX_KEY: &str = "__row";

/// CRUD over a Google Sheets workbook. One instance per
/// `(service_account, default_spreadsheet_id)` pairing. Holds:
/// - the reqwest blocking client (rustls-tls, reused for keep-alive),
/// - the parsed service-account credentials,
/// - a cached access token (lazy refresh near expiry),
/// - a per-(spreadsheet, tab) header cache so writes don't re-read row 1.
///
/// All methods are `&self` — refresh + cache state lives behind
/// `RefCell`s so callers don't need a mut binding.
pub struct SheetsRepository {
    http: reqwest::blocking::Client,
    sa: ServiceAccount,
    pub default_spreadsheet_id: String,
    token: RefCell<Option<AccessToken>>,
    header_cache: RefCell<HashMap<String, Vec<String>>>,
}

impl SheetsRepository {
    /// Build a repository from a default spreadsheet id and the raw
    /// service-account JSON. The JSON is parsed once and held;
    /// network is only touched on first use of a method that needs a
    /// token.
    pub fn new(
        default_spreadsheet_id: String,
        service_account_json: &str,
    ) -> Result<Self, SheetsError> {
        let sa = ServiceAccount::from_json(service_account_json)?;
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(SheetsError::from)?;
        Ok(Self {
            http,
            sa,
            default_spreadsheet_id,
            token: RefCell::new(None),
            header_cache: RefCell::new(HashMap::new()),
        })
    }

    fn spreadsheet_id_for(&self, view: &ViewSchema) -> String {
        view.spreadsheet_id
            .clone()
            .unwrap_or_else(|| self.default_spreadsheet_id.clone())
    }

    fn cache_key(&self, view: &ViewSchema) -> String {
        format!("{}|{}", self.spreadsheet_id_for(view), view.table)
    }

    /// Lazily refresh and return the bearer-token string.
    fn token_value(&self) -> Result<String, SheetsError> {
        let needs_refresh = self
            .token
            .borrow()
            .as_ref()
            .map_or(true, |t| !t.is_fresh());
        if needs_refresh {
            let new_token = retry(|| fetch_access_token(&self.sa, &self.http))?;
            *self.token.borrow_mut() = Some(new_token);
        }
        Ok(self
            .token
            .borrow()
            .as_ref()
            .expect("set above")
            .token
            .clone())
    }

    /// Re-callable HTTP wrapper. Retries on transport errors (matches
    /// the Dart `_RetryingClient`'s "only retry exceptions, never
    /// retry response codes" rule, so non-idempotent writes stay safe)
    /// AND treats a 401 from the Sheets API as "token might be stale"
    /// — clears the cached token and retries once with a fresh one.
    fn api<R, F>(&self, f: F) -> Result<R, SheetsError>
    where
        F: Fn(&Api) -> Result<R, SheetsError>,
    {
        let mut attempt = 0;
        let mut auth_retried = false;
        loop {
            attempt += 1;
            let token = self.token_value()?;
            let api = Api {
                http: &self.http,
                token: &token,
            };
            match f(&api) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    // 401 with a cached token: assume the token went
                    // bad mid-flight (deep-sleep clock skew, server-
                    // side revoke, etc). Invalidate + retry once.
                    if !auth_retried
                        && matches!(&e, SheetsError::Api { status: 401, .. })
                    {
                        *self.token.borrow_mut() = None;
                        auth_retried = true;
                        continue;
                    }
                    if attempt >= RETRY_ATTEMPTS || !is_transient(&e) {
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(
                        RETRY_BASE_MS * (1 << (attempt - 1)),
                    ));
                }
            }
        }
    }

    /// Ensures the sheet tab exists and has every header the view
    /// expects. Additive — preserves existing headers and their order,
    /// only appends missing columns at the end. Safe against pre-
    /// existing sheets with extra columns the schema doesn't know about.
    pub fn ensure_sheet(&self, view: &ViewSchema) -> Result<(), SheetsError> {
        let spreadsheet_id = self.spreadsheet_id_for(view);

        let meta = self.api(|api| api.get_spreadsheet(&spreadsheet_id))?;
        let tab_exists = meta
            .sheets
            .iter()
            .any(|s| s.properties.title == view.table);

        if !tab_exists {
            self.api(|api| {
                api.batch_update(
                    &spreadsheet_id,
                    vec![serde_json::json!({
                        "addSheet": { "properties": { "title": view.table } }
                    })],
                )
            })?;
        }

        let header_range = format!("'{}'!1:1", view.table);
        let hdr = self.api(|api| api.get_values(&spreadsheet_id, &header_range))?;
        let existing_headers: Vec<String> = hdr
            .values
            .first()
            .map(|row| row.iter().map(value_to_string).collect())
            .unwrap_or_default();
        let want_headers: Vec<String> =
            view.dimensions.iter().map(|d| d.expr.clone()).collect();
        let missing: Vec<String> = want_headers
            .iter()
            .filter(|h| !existing_headers.contains(h))
            .cloned()
            .collect();

        if !existing_headers.is_empty() && missing.is_empty() {
            self.header_cache
                .borrow_mut()
                .insert(self.cache_key(view), existing_headers);
            return Ok(());
        }

        let new_headers: Vec<String> = if existing_headers.is_empty() {
            want_headers
        } else {
            let mut h = existing_headers;
            h.extend(missing);
            h
        };

        let update_range = format!("'{}'!A1", view.table);
        let row: Vec<Value> = new_headers
            .iter()
            .map(|h| Value::String(h.clone()))
            .collect();
        self.api(|api| api.update_values(&spreadsheet_id, &update_range, row.clone()))?;
        self.header_cache
            .borrow_mut()
            .insert(self.cache_key(view), new_headers);
        Ok(())
    }

    /// List every data row. When `on_date` is given AND the view has
    /// a `date_field`, only rows on that date are returned, sorted by
    /// the plannable log field's time-of-day (so the morning set
    /// appears before the evening set even when sheets has them in
    /// insertion order).
    pub fn list(
        &self,
        view: &ViewSchema,
        on_date: Option<NaiveDate>,
    ) -> Result<Vec<Record>, SheetsError> {
        let spreadsheet_id = self.spreadsheet_id_for(view);
        let range = format!("'{}'", view.table);
        let vals = self.api(|api| api.get_values(&spreadsheet_id, &range))?;

        if vals.values.is_empty() {
            return Ok(vec![]);
        }
        let headers: Vec<String> = vals.values[0].iter().map(value_to_string).collect();
        self.header_cache
            .borrow_mut()
            .insert(self.cache_key(view), headers.clone());

        let mut records: Vec<Record> = vec![];
        for (i, row) in vals.values.iter().enumerate().skip(1) {
            let mut record = row_to_record(view, &headers, row);
            record.insert(
                ROW_INDEX_KEY.to_string(),
                CellValue::Int((i as i64) - 1),
            );
            records.push(record);
        }

        let Some(on_date) = on_date else {
            return Ok(records);
        };
        let Some(date_field) = view.date_field.clone() else {
            return Ok(records);
        };

        let mut filtered: Vec<Record> = records
            .into_iter()
            .filter(|r| matches!(r.get(&date_field), Some(CellValue::Date(d)) if *d == on_date))
            .collect();

        let log_field = view.plannable.as_ref().map(|p| p.log_field.clone());
        filtered.sort_by(|a, b| {
            if let Some(ref lf) = log_field {
                let av = a.get(lf).map(|v| v.to_display_string()).unwrap_or_default();
                let bv = b.get(lf).map(|v| v.to_display_string()).unwrap_or_default();
                match (av.is_empty(), bv.is_empty()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    (false, false) => match (parse_time(&av), parse_time(&bv)) {
                        (Some(a), Some(b)) => a.cmp(&b),
                        _ => av.cmp(&bv),
                    },
                }
            } else {
                row_index(a).cmp(&row_index(b))
            }
        });
        Ok(filtered)
    }

    /// Insert a new record at sheet row 2 (top of data). Auto-assigns
    /// a UUID to the `id` field if the view declares one and the
    /// record doesn't already have one. Returned record has
    /// `__row = 0` so it stays addressable without a re-read.
    ///
    /// Every previously loaded record's `__row` becomes stale by -1
    /// after this call — callers maintaining an in-memory list must
    /// call [`shift_row_indexes(records, by: 1)`].
    pub fn create(
        &self,
        view: &ViewSchema,
        record: Record,
    ) -> Result<Record, SheetsError> {
        let spreadsheet_id = self.spreadsheet_id_for(view);
        let headers = self.ensure_headers(view)?;
        let mut to_write = record;
        if view.dimension_by_name("id").is_some() && !to_write.contains_key("id") {
            to_write.insert("id".into(), CellValue::String(Uuid::new_v4().to_string()));
        }

        let row: Vec<Value> = headers
            .iter()
            .map(|h| {
                view.dimension_by_expr(h)
                    .map(|dim| {
                        let raw = to_write
                            .get(&dim.name)
                            .cloned()
                            .unwrap_or(CellValue::Null);
                        cell_to_json(encode(dim.kind, &raw))
                    })
                    .unwrap_or(Value::String(String::new()))
            })
            .collect();

        let sheet_id = self.sheet_id_for(&spreadsheet_id, &view.table)?;
        self.api(|api| {
            api.batch_update(
                &spreadsheet_id,
                vec![serde_json::json!({
                    "insertDimension": {
                        "range": {
                            "sheetId": sheet_id,
                            "dimension": "ROWS",
                            "startIndex": 1,
                            "endIndex": 2,
                        },
                        "inheritFromBefore": false,
                    }
                })],
            )
        })?;

        let range = format!("'{}'!A2", view.table);
        self.api(|api| api.update_values(&spreadsheet_id, &range, row.clone()))?;
        to_write.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(0));
        Ok(to_write)
    }

    /// Update an existing record. Resolves the row by `__row` if set,
    /// otherwise by `id`. Preserves any sheet columns the schema
    /// doesn't know about (verbatim — whatever the API returned).
    pub fn update(
        &self,
        view: &ViewSchema,
        mut record: Record,
    ) -> Result<(), SheetsError> {
        let spreadsheet_id = self.spreadsheet_id_for(view);
        let headers = self.ensure_headers(view)?;

        let row_index = if let Some(CellValue::Int(idx)) = record.get(ROW_INDEX_KEY) {
            *idx as usize
        } else {
            let id = record
                .get("id")
                .map(|v| v.to_display_string())
                .filter(|s| !s.is_empty())
                .ok_or(SheetsError::NoRowRef)?;
            self.find_row_index(view, &id)?
                .ok_or_else(|| SheetsError::IdNotFound(id, view.table.clone()))?
        };

        if view.dimension_by_name("id").is_some() {
            let needs_id = record
                .get("id")
                .map(|v| v.is_empty())
                .unwrap_or(true);
            if needs_id {
                record.insert("id".into(), CellValue::String(Uuid::new_v4().to_string()));
            }
        }

        let existing = self.read_row(&spreadsheet_id, &view.table, row_index)?;

        let row: Vec<Value> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| {
                if let Some(dim) = view.dimension_by_expr(h) {
                    let raw = record
                        .get(&dim.name)
                        .cloned()
                        .unwrap_or(CellValue::Null);
                    cell_to_json(encode(dim.kind, &raw))
                } else {
                    existing.get(i).cloned().unwrap_or(Value::String(String::new()))
                }
            })
            .collect();

        let range = format!("'{}'!A{}", view.table, row_index + 2);
        self.api(|api| api.update_values(&spreadsheet_id, &range, row.clone()))?;
        Ok(())
    }

    /// Delete a record's sheet row. Silently no-ops if the row can't
    /// be resolved (mirrors Dart behavior — clean delete semantics
    /// for retried calls).
    pub fn delete(
        &self,
        view: &ViewSchema,
        record: &Record,
    ) -> Result<(), SheetsError> {
        let spreadsheet_id = self.spreadsheet_id_for(view);
        let row_index = if let Some(CellValue::Int(idx)) = record.get(ROW_INDEX_KEY) {
            Some(*idx as usize)
        } else if let Some(id) = record.get("id") {
            let s = id.to_display_string();
            if s.is_empty() {
                None
            } else {
                self.find_row_index(view, &s)?
            }
        } else {
            None
        };
        let Some(row_index) = row_index else {
            return Ok(());
        };
        let sheet_id = self.sheet_id_for(&spreadsheet_id, &view.table)?;
        self.api(|api| {
            api.batch_update(
                &spreadsheet_id,
                vec![serde_json::json!({
                    "deleteDimension": {
                        "range": {
                            "sheetId": sheet_id,
                            "dimension": "ROWS",
                            "startIndex": row_index + 1,
                            "endIndex": row_index + 2,
                        }
                    }
                })],
            )
        })?;
        Ok(())
    }

    // --- internals ---

    fn ensure_headers(&self, view: &ViewSchema) -> Result<Vec<String>, SheetsError> {
        let key = self.cache_key(view);
        if let Some(h) = self.header_cache.borrow().get(&key).cloned() {
            return Ok(h);
        }
        let spreadsheet_id = self.spreadsheet_id_for(view);
        let range = format!("'{}'!1:1", view.table);
        let vals = self.api(|api| api.get_values(&spreadsheet_id, &range))?;
        let headers: Vec<String> = vals
            .values
            .first()
            .map(|row| row.iter().map(value_to_string).collect())
            .unwrap_or_default();
        self.header_cache.borrow_mut().insert(key, headers.clone());
        Ok(headers)
    }

    fn read_row(
        &self,
        spreadsheet_id: &str,
        table: &str,
        row_index: usize,
    ) -> Result<Vec<Value>, SheetsError> {
        let range = format!("'{}'!{}:{}", table, row_index + 2, row_index + 2);
        let vals = self.api(|api| api.get_values(spreadsheet_id, &range))?;
        Ok(vals.values.into_iter().next().unwrap_or_default())
    }

    fn find_row_index(
        &self,
        view: &ViewSchema,
        id: &str,
    ) -> Result<Option<usize>, SheetsError> {
        let spreadsheet_id = self.spreadsheet_id_for(view);
        let Some(id_dim) = view.dimension_by_name("id") else {
            return Ok(None);
        };
        let range = format!("'{}'", view.table);
        let vals = self.api(|api| api.get_values(&spreadsheet_id, &range))?;
        if vals.values.is_empty() {
            return Ok(None);
        }
        let headers: Vec<String> = vals.values[0].iter().map(value_to_string).collect();
        let Some(id_col) = headers.iter().position(|h| h == &id_dim.expr) else {
            return Ok(None);
        };
        for (i, row) in vals.values.iter().enumerate().skip(1) {
            if row.get(id_col).map(|v| value_to_string(v) == id).unwrap_or(false) {
                return Ok(Some(i - 1));
            }
        }
        Ok(None)
    }

    fn sheet_id_for(&self, spreadsheet_id: &str, tab_name: &str) -> Result<i64, SheetsError> {
        let meta = self.api(|api| api.get_spreadsheet(spreadsheet_id))?;
        meta.sheets
            .iter()
            .find(|s| s.properties.title == tab_name)
            .map(|s| s.properties.sheet_id)
            .ok_or_else(|| SheetsError::MissingTab(tab_name.to_string()))
    }
}

/// Shift the `__row` index on every record by `by`. Call with `by: 1`
/// after a successful [`SheetsRepository::create`] so previously
/// loaded records stay addressable for subsequent update / delete.
pub fn shift_row_indexes(records: &mut [Record], by: i64) {
    for r in records {
        if let Some(CellValue::Int(idx)) = r.get(ROW_INDEX_KEY).cloned() {
            r.insert(ROW_INDEX_KEY.to_string(), CellValue::Int(idx + by));
        }
    }
}

fn row_to_record(view: &ViewSchema, headers: &[String], row: &[Value]) -> Record {
    let mut record = Record::new();
    for (i, h) in headers.iter().enumerate() {
        let Some(dim) = view.dimension_by_expr(h) else {
            continue;
        };
        let raw = row.get(i).map(value_to_string).unwrap_or_default();
        record.insert(dim.name.clone(), decode(dim.kind, &raw));
    }
    record
}

fn row_index(r: &Record) -> i64 {
    match r.get(ROW_INDEX_KEY) {
        Some(CellValue::Int(n)) => *n,
        _ => 0,
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

fn cell_to_json(v: CellValue) -> Value {
    match v {
        CellValue::Null => Value::String(String::new()),
        CellValue::Bool(b) => Value::Bool(b),
        CellValue::Int(n) => Value::Number(n.into()),
        CellValue::Float(n) => serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        // Strings starting with `=` would be interpreted as formulas
        // by the USER_ENTERED value-input option. Prefix with `'` so
        // Sheets stores them as literal text (the apostrophe gets
        // stripped on display). Date/DateTime formats below never
        // start with `=`, so they don't need the guard.
        CellValue::String(s) => Value::String(escape_formula(s)),
        CellValue::Date(d) => Value::String(d.format("%Y-%m-%d").to_string()),
        CellValue::DateTime(dt) => Value::String(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
    }
}

fn escape_formula(s: String) -> String {
    if s.starts_with('=') {
        format!("'{s}")
    } else {
        s
    }
}

/// Parse a time-of-day string into a `NaiveTime` for chronological
/// sort. Tries 12-hour (`h:mm:ss AM`, `h:mm AM`) then 24-hour
/// (`H:mm:ss`, `H:mm`) — matches the formats `_parseTime` in
/// `sheets_repository.dart` recognizes.
fn parse_time(s: &str) -> Option<NaiveTime> {
    for fmt in &[
        "%-I:%M:%S %p",
        "%-I:%M %p",
        "%I:%M:%S %p",
        "%I:%M %p",
        "%H:%M:%S",
        "%H:%M",
    ] {
        if let Ok(t) = NaiveTime::parse_from_str(s, fmt) {
            return Some(t);
        }
    }
    None
}

/// Retry `f` up to `RETRY_ATTEMPTS` times with exponential backoff,
/// only retrying transport-level failures. Server-side errors
/// (4xx / 5xx) come back immediately so non-idempotent writes stay
/// safe — matches the Dart `_RetryingClient` policy.
fn retry<T, F: FnMut() -> Result<T, SheetsError>>(mut f: F) -> Result<T, SheetsError> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if attempt >= RETRY_ATTEMPTS || !is_transient(&e) => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(
                RETRY_BASE_MS * (1 << (attempt - 1)),
            )),
        }
    }
}

fn is_transient(e: &SheetsError) -> bool {
    let SheetsError::Http(err) = e else { return false };
    if err.is_connect() || err.is_timeout() {
        return true;
    }
    // reqwest's typed predicates miss "request was sent but the
    // socket was aborted mid-flight" — that's a Request-kind error
    // with the real cause buried in the source chain. Walk it and
    // look for the well-known cold-start race patterns the Dart
    // `_RetryingClient` already covered.
    let mut source: Option<&dyn std::error::Error> = Some(err);
    while let Some(s) = source {
        let msg = s.to_string().to_lowercase();
        if msg.contains("connection abort")
            || msg.contains("connection closed")
            || msg.contains("connection reset")
            || msg.contains("connection refused")
            || msg.contains("connection terminated")
            || msg.contains("broken pipe")
            || msg.contains("handshake")
            || msg.contains("software caused connection abort")
            || msg.contains("unexpected eof")
            || msg.contains("os error 104") // ECONNRESET
            || msg.contains("os error 32")  // EPIPE
        {
            return true;
        }
        source = s.source();
    }
    false
}
