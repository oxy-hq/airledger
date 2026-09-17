# Kaya → climbing Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pull the user's full Kaya logbook into a new per-ascent `climbing` view via a background-pull integration, on top of a new engine `match_field` ingest mode.

**Architecture:** Engine ingest gains an optional `match_field` (rows match by a dimension like `kaya_id` instead of by date) plus `deleted_ids` unwind. A new `climbing` view/input schema flows to Sheets like any tracker. `KayaIntegration` (Withings pattern) logs into Kaya's unofficial GraphQL API with email/password, walks the full ascent list every pull (no cursor — sort order of the API is unverified, and a full walk is ~5 requests; correctness never depends on order), and ingests with `match_field: kaya_id`. Deletions = known ids minus fetched ids.

**Tech Stack:** Rust (airledger engine, serde/rusqlite), Dart/Flutter (ledger app, http + flutter_secure_storage), YAML schemas (airledger-fitness).

**Spec:** `docs/superpowers/specs/2026-09-16-kaya-integration-design.md` (this repo). One deliberate deviation, allowed by the spec's step-2 fallback: there is NO incremental cursor — every pull is a full walk + full reconcile, because the API's sort order is unverified and the logbook is small. `fullReconcile` therefore behaves identically to a normal pull.

**Repos:** Tasks 1–3: `~/repos/airledger` · Task 4: `~/repos/airledger-fitness` · Tasks 5–10: `~/repos/ledger`. Commit conventions everywhere: conventional style + trailer `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

**Baselines:** `cargo test` in airledger must stay green. In ledger, `flutter test` has exactly 7 known failures (3 live-DB integration suites + 4 schema_loader) and `flutter analyze` ~32 infos — anything beyond that is a regression you introduced.

---

### Task 1: Engine — `match_field` upsert mode

**Files:**
- Modify: `~/repos/airledger/src/store/ingest.rs`
- Test: `~/repos/airledger/tests/ingest_unit.rs`

- [ ] **Step 1: Write the failing tests**

Append to `tests/ingest_unit.rs` (mirror the existing helpers at the top of that file — `temp_store`, `batch`):

```rust
fn climbing_view() -> airledger_engine::ViewSchema {
    let base = parse_view(
        "name: climbing\ndatasource: gsheets\ntable: climbing\ndimensions:\n  - { name: id, type: string, expr: id }\n  - { name: kaya_id, type: string, expr: kaya_id }\n  - { name: date, type: date, expr: date }\n  - { name: climb_name, type: string, expr: climb_name }\n  - { name: grade, type: string, expr: grade }\n  - { name: notes, type: string, expr: notes }\n",
    )
    .unwrap();
    let overlay =
        parse_input_overlay("target: climbing.view.yml\ndate_field: date\n").unwrap();
    apply_overlay(base, overlay).unwrap()
}

const ASCENTS_BATCH: &str = r#"{
  "source": "kaya",
  "match_field": "kaya_id",
  "owned_fields": ["kaya_id", "date", "climb_name", "grade"],
  "fill_if_blank_fields": ["notes"],
  "records": [
    {"kaya_id":{"kind":"string","value":"a1"},"date":{"kind":"date","value":"2026-09-14"},"climb_name":{"kind":"string","value":"Moonwalk"},"grade":{"kind":"string","value":"V5"}},
    {"kaya_id":{"kind":"string","value":"a2"},"date":{"kind":"date","value":"2026-09-14"},"climb_name":{"kind":"string","value":"Slab City"},"grade":{"kind":"string","value":"V3"}}
  ]
}"#;

#[test]
fn match_field_creates_multiple_rows_on_one_day() {
    let store = temp_store("mf-create");
    let view = climbing_view();
    let res = ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.skipped), (2, 0, 0));
    assert_eq!(store.list(&view, None).unwrap().len(), 2, "same day, two rows");
}

#[test]
fn match_field_replay_is_noop() {
    let store = temp_store("mf-replay");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    let res = ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 0, 2));
}

#[test]
fn match_field_upserts_by_id_even_when_date_changes() {
    let store = temp_store("mf-upsert");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    // Kaya revises a2: new grade AND moved to another day.
    let revised = ASCENTS_BATCH.replace("V3", "V4").replace("2026-09-14\"},\"climb_name\":{\"kind\":\"string\",\"value\":\"Slab City", "2026-09-15\"},\"climb_name\":{\"kind\":\"string\",\"value\":\"Slab City");
    let res = ingest(&store, &view, &batch(&revised)).unwrap();
    assert_eq!((res.created, res.updated, res.unchanged), (0, 1, 1));
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 2, "revision matched by id, no duplicate row");
}

#[test]
fn match_field_leaves_manual_rows_alone() {
    let store = temp_store("mf-manual");
    let view = climbing_view();
    // Hand-entered row, same day, no kaya_id.
    let mut manual = std::collections::BTreeMap::new();
    manual.insert(
        "date".to_string(),
        CellValue::Date(chrono::NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()),
    );
    manual.insert("climb_name".to_string(), CellValue::String("Project X".into()));
    store.create(&view, manual).unwrap();

    let res = ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    assert_eq!(res.created, 2, "manual row never matches; batch rows created fresh");
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|r| r.get("climb_name")
        == Some(&CellValue::String("Project X".into()))));
}

#[test]
fn match_field_record_without_key_is_skipped() {
    let store = temp_store("mf-nokey");
    let view = climbing_view();
    let b = batch(
        r#"{"source":"kaya","match_field":"kaya_id","records":[{"date":{"kind":"date","value":"2026-09-14"},"grade":{"kind":"string","value":"V1"}}]}"#,
    );
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.skipped, 1);
    assert!(store.list(&view, None).unwrap().is_empty());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/repos/airledger && cargo test --test ingest_unit 2>&1 | tail -20`
Expected: compile error — `IngestBatch` has no field `match_field` (serde: unknown field). That is the correct RED.

- [ ] **Step 3: Implement `match_field` in `src/store/ingest.rs`**

Add the two fields to `IngestBatch`:

```rust
#[derive(Debug, Deserialize)]
pub struct IngestBatch {
    pub source: String,
    /// When set, rows match records by this dimension's value instead of
    /// by `date_field` — row-grained sources (one row per ascent) rather
    /// than day-grained ones. Rows with an empty value for the field are
    /// invisible to the batch (hand-entered rows are never touched).
    #[serde(default)]
    pub match_field: Option<String>,
    #[serde(default)]
    pub owned_fields: Vec<String>,
    #[serde(default)]
    pub fill_if_blank_fields: Vec<String>,
    #[serde(default)]
    pub records: Vec<Record>,
    #[serde(default)]
    pub deleted_dates: Vec<String>,
    /// Unwind list for `match_field` mode (values of that field). Used
    /// instead of `deleted_dates` when `match_field` is set.
    #[serde(default)]
    pub deleted_ids: Vec<String>,
}
```

In `ingest()`, generalize the day-keying to key-keying. `date_field` stays required (rows still live on the timeline). Replace the `by_date` construction and the per-record `day` lookup:

```rust
    let date_field = view
        .date_field
        .clone()
        .ok_or_else(|| StoreError::NotFound("date_field".into(), view.name.clone()))?;
    let key_field = batch.match_field.clone().unwrap_or_else(|| date_field.clone());
    store.tx(|s| {
        let mut res = IngestResult::default();
        // Index live rows by the key field's display string. In date
        // mode the first row of the day wins (one-row-per-day views);
        // rows with an empty key never match (in match_field mode these
        // are the hand-entered rows).
        let rows = s.list(view, None)?;
        let mut by_key: BTreeMap<String, Record> = BTreeMap::new();
        for r in rows {
            let k = r
                .get(&key_field)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            if k.is_empty() {
                continue;
            }
            by_key.entry(k).or_insert(r);
        }

        for rec in &batch.records {
            let key = rec
                .get(&key_field)
                .map(|v| v.to_display_string())
                .unwrap_or_default();
            if key.is_empty() {
                res.skipped += 1;
                continue;
            }
            match by_key.get(&key).cloned() {
```

…and rename the remaining `day`/`by_date` occurrences in the match arms to `key`/`by_key` (`by_key.insert(key, created)`, `by_key.insert(key, updated)`). The provenance logic is untouched — it keys on row `id`, not on the match key.

Note the pre-existing behavior is preserved: empty-date rows used to be indexed under `""`, but records with an empty date were skipped and `deleted_dates` never contains `""`, so skipping empty keys is observably identical in date mode.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd ~/repos/airledger && cargo test --test ingest_unit 2>&1 | tail -5`
Expected: all tests pass, including every pre-existing date-mode test (Withings semantics unchanged).

- [ ] **Step 5: Run the full engine suite**

Run: `cd ~/repos/airledger && cargo test 2>&1 | tail -5`
Expected: green.

- [ ] **Step 6: Commit**

```bash
cd ~/repos/airledger
git add src/store/ingest.rs tests/ingest_unit.rs
git commit -m "feat(ingest): match_field mode — upsert rows by a dimension value

Row-grained sources (Kaya ascents: many rows per day) match records to
rows by e.g. kaya_id instead of date_field. Date mode is untouched.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: Engine — `deleted_ids` unwind

**Files:**
- Modify: `~/repos/airledger/src/store/ingest.rs` (apply_deletions)
- Test: `~/repos/airledger/tests/ingest_unit.rs`

- [ ] **Step 1: Write the failing tests**

Append to `tests/ingest_unit.rs`:

```rust
#[test]
fn deleted_id_removes_source_created_untouched_row() {
    let store = temp_store("mf-del");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    let b = batch(r#"{"source":"kaya","match_field":"kaya_id","deleted_ids":["a2"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!(res.deleted, 1);
    let rows = store.list(&view, None).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("kaya_id"), Some(&CellValue::String("a1".into())));
}

#[test]
fn deleted_id_with_user_edit_clears_source_fields_keeps_edit() {
    let store = temp_store("mf-del-edit");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    // User renames a2's climb.
    let mut row = store
        .list(&view, None)
        .unwrap()
        .into_iter()
        .find(|r| r.get("kaya_id") == Some(&CellValue::String("a2".into())))
        .unwrap();
    row.insert("climb_name".to_string(), CellValue::String("My Name".into()));
    store.update(&view, row).unwrap();

    let b = batch(r#"{"source":"kaya","match_field":"kaya_id","deleted_ids":["a2"]}"#);
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 1));
    let row = store
        .list(&view, None)
        .unwrap()
        .into_iter()
        .find(|r| r.get("climb_name") == Some(&CellValue::String("My Name".into())))
        .expect("edited row survives");
    assert_eq!(row.get("grade"), Some(&CellValue::Null), "source field cleared");
    assert_eq!(
        row.get("kaya_id"),
        Some(&CellValue::String("a2".into())),
        "identity fields exempt from clearing"
    );
}

#[test]
fn deleted_dates_are_ignored_in_match_field_mode() {
    let store = temp_store("mf-del-dates");
    let view = climbing_view();
    ingest(&store, &view, &batch(ASCENTS_BATCH)).unwrap();
    let b = batch(
        r#"{"source":"kaya","match_field":"kaya_id","deleted_dates":["2026-09-14"]}"#,
    );
    let res = ingest(&store, &view, &b).unwrap();
    assert_eq!((res.deleted, res.cleared), (0, 0));
    assert_eq!(store.list(&view, None).unwrap().len(), 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd ~/repos/airledger && cargo test --test ingest_unit 2>&1 | tail -20`
Expected: the three new tests FAIL (`deleted_ids` currently ignored / `deleted_dates` incorrectly keyed against the id index). Everything else passes.

- [ ] **Step 3: Implement**

In `ingest()`, replace the `apply_deletions(...)` call with mode selection:

```rust
        // In match_field mode the unwind list is deleted_ids; the index
        // is keyed by that field, so deleted_dates would be meaningless
        // (and vice versa).
        let unwind: &[String] = if batch.match_field.is_some() {
            &batch.deleted_ids
        } else {
            &batch.deleted_dates
        };
        apply_deletions(s, view, batch, &date_field, &key_field, unwind, &mut by_key, &mut res)?;
```

Update `apply_deletions` to take the key list + key field and exempt BOTH identity fields in the clear branch:

```rust
fn apply_deletions(
    s: &Store,
    view: &ViewSchema,
    batch: &IngestBatch,
    date_field: &str,
    key_field: &str,
    unwind: &[String],
    by_key: &mut BTreeMap<String, Record>,
    res: &mut IngestResult,
) -> Result<(), StoreError> {
    for key in unwind {
        let Some(row) = by_key.get(key).cloned() else {
            continue;
        };
        // ... body unchanged except the clear-branch exemption:
        //     if f == date_field { continue; }
        // becomes
        //     if f == date_field || f == key_field {
        //         continue; // row identity fields survive the clear
        //     }
        // and the trailing `by_date` ops become `by_key`.
    }
    Ok(())
}
```

(The rest of the body — provenance lookup, untouched check, delete vs clear, `provenance_remove` — is copied verbatim from the current function with `day` renamed to `key`.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd ~/repos/airledger && cargo test 2>&1 | tail -5`
Expected: full suite green (date-mode deletion tests confirm Withings unwind still works).

- [ ] **Step 5: Commit**

```bash
cd ~/repos/airledger
git add src/store/ingest.rs tests/ingest_unit.rs
git commit -m "feat(ingest): deleted_ids unwind for match_field mode

Same provenance rules as deleted_dates: source-created + untouched
rows delete; edited rows keep user values, source fields clear,
identity fields (date_field, match_field) exempt.

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: Engine — rebuild the Android dylib (TRAP #1)

**Files:** none in git (build artifact into the ledger app's jniLibs)

- [ ] **Step 1: Build**

Run: `cd ~/repos/airledger/sdk-dart && ./scripts/build-android.sh`
Expected: successful cross-compile; script copies `libairledger_engine.so` into the ledger app's jniLibs.

- [ ] **Step 2: Sanity-check the new symbols are in the .so**

Run: `strings ~/repos/ledger/android/app/src/main/jniLibs/arm64-v8a/libairledger_engine.so | grep -E "match_field|deleted_ids"`
(If the jniLibs path differs, find it: `find ~/repos/ledger -name libairledger_engine.so`.)
Expected: both strings print. If not, the app would silently drop the new batch keys — do not proceed until this passes.

---

### Task 4: Schemas — `climbing` view (TRAP #2: must be pushed)

**Files:**
- Create: `~/repos/airledger-fitness/views/climbing.view.yml`
- Create: `~/repos/airledger-fitness/views/climbing.input.yml`

- [ ] **Step 1: Write `views/climbing.view.yml`**

```yaml
name: climbing
description: "Climbing log — one row per ascent. Synced from Kaya (kaya_id ties a row to its Kaya ascent; hand-added rows leave it blank and the sync never touches them)."
datasource: gsheets
table: climbing
entities:
  - { name: ascent, type: primary, key: id }
dimensions:
  - { name: id, type: string, expr: id, description: Unique row identifier (UUID) }
  - { name: kaya_id, type: string, expr: kaya_id, description: Kaya ascent id (blank for manual rows) }
  - { name: date, type: date, expr: date, description: Ascent day }
  - { name: climb_name, type: string, expr: climb_name }
  - { name: climb_type, type: string, expr: climb_type, description: boulder or route }
  - { name: grade, type: string, expr: grade, description: "Kaya grade string verbatim (V5, 5.12a)" }
  - { name: ascent_type, type: string, expr: ascent_type, description: flash / redpoint / onsight / send }
  - { name: attempts, type: number, expr: attempts }
  - { name: lead, type: boolean, expr: lead, description: Led the route (routes only) }
  - { name: gym, type: string, expr: gym, description: Gym name (blank when outdoor) }
  - { name: location, type: string, expr: location, description: Outdoor destination/area (blank in the gym) }
  - { name: notes, type: string, expr: notes, description: Kaya comment; safe to annotate — user edits survive pulls }
measures:
  - { name: ascent_count, type: count }
```

- [ ] **Step 2: Write `views/climbing.input.yml`**

First check how the icon name resolves so it doesn't fall back silently: `grep -rn "'scale'" ~/repos/ledger/lib/ | head -3` and pick a name that exists in that map (e.g. `terrain` or `mountain` — whichever the resolver knows; `scale` proves the format).

```yaml
target: climbing.view.yml
icon: terrain
date_field: date
list_display:
  title: climb_name
  subtitle: ${grade} · ${ascent_type}
fields:
  id:
    editable: false
  kaya_id:
    editable: false
  date:
    widget: date
    default: today
    required: true
  climb_name:
    widget: text
  climb_type:
    widget: dropdown
    options: [boulder, route]
  grade:
    widget: text
    placeholder: 'V5 / 5.12a'
  ascent_type:
    widget: dropdown
    options: [flash, redpoint, onsight, send]
  attempts:
    widget: number
    min: 1
  lead:
    editable: false
  gym:
    widget: text
  location:
    widget: text
  notes:
    widget: longtext
```

(`lead` is display-only: there is no boolean form widget today and ingest writes it regardless of the overlay. YAGNI — add a widget if hand-editing it ever matters.)

- [ ] **Step 3: Sanity-parse through the engine**

Run: `cd ~/repos/airledger && cargo run --quiet --example parse_check ~/repos/airledger-fitness/views/climbing.view.yml 2>/dev/null || cargo test 2>&1 | tail -3`
(If no parse_check example exists, skip — Task 9's `flutter test` + on-device load covers it; the view uses only constructs already present in weight/strength views.)

- [ ] **Step 4: Commit AND PUSH (Trap #2 — an unpushed view gets reverted on device within ~5 min)**

```bash
cd ~/repos/airledger-fitness
git add views/climbing.view.yml views/climbing.input.yml
git commit -m "feat: climbing view — per-ascent log fed by the Kaya integration

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
git push
```

---

### Task 5: App — verify the live Kaya API contract

The GraphQL details come from community reverse-engineering (`Asherlc/dofek`). Pin them down before coding against them.

- [ ] **Step 1: Fetch the observed API contract**

Fetch (WebFetch or curl) — try `main` then `master` branches:
- `https://raw.githubusercontent.com/Asherlc/dofek/main/docs/kaya-api.openapi.yaml`
- `https://raw.githubusercontent.com/Asherlc/dofek/main/packages/kaya-client/src/client.ts`

Extract and write down: (a) exact login/refresh paths and response field names, (b) the exact `ascentsForUser` GraphQL query text (field names, whether outdoor location is `destination`/`area`/on `climb`), (c) the wire format of the ascent `date` field (ISO string vs epoch), (d) required headers. Fallback if the repo is gone: `https://raw.githubusercontent.com/betabook-ca/betabook/main/docs/kaya-import.md`.

- [ ] **Step 2: Reconcile with the code in Tasks 6–7**

Tasks 6–7 below encode the researched contract. Where step 1 disagrees, the fetched contract wins — update the query constant, response parsing, and `kayaDay()` accordingly, and adjust test fixtures to match. If the contract can't be verified at all, keep the code as written (it is the best documented shape) and rely on the Task 10 on-device check with tolerant parsing.

---

### Task 6: App — `KayaApi` HTTP client

**Files:**
- Create: `~/repos/ledger/lib/services/integrations/kaya_api.dart`
- Test: `~/repos/ledger/test/kaya_api_test.dart`

- [ ] **Step 1: Write the failing tests**

```dart
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

import 'package:airledger/services/integrations/kaya_api.dart';

void main() {
  test('login returns tokens and user id', () async {
    final api = KayaApi(
      client: MockClient((req) async {
        expect(req.url.toString(),
            'https://kaya-beta.kayaclimb.com/api/user/login');
        expect(req.headers['Origin'], 'https://kaya-app.kayaclimb.com');
        final body = jsonDecode(req.body) as Map<String, dynamic>;
        expect(body['email'], 'a@b.c');
        return http.Response(
            jsonEncode({
              'message': 'ok',
              'token': 't1',
              'refresh_token': 'r1',
              'user': {'id': 42},
            }),
            200);
      }),
    );
    final auth = await api.login('a@b.c', 'pw');
    expect((auth.token, auth.refreshToken, auth.userId), ('t1', 'r1', '42'));
  });

  test('login surfaces bad credentials as an error', () async {
    final api = KayaApi(
        client: MockClient((_) async => http.Response('{"message":"nope"}', 401)));
    expect(() => api.login('a@b.c', 'bad'), throwsA(isA<StateError>()));
  });

  test('ascentsPage sends bearer + query and parses the list', () async {
    final api = KayaApi(
      client: MockClient((req) async {
        expect(req.url.path, '/graphql');
        expect(req.headers['Authorization'], 'Bearer t1');
        final body = jsonDecode(req.body) as Map<String, dynamic>;
        expect(body['query'], contains('ascentsForUser'));
        expect(body['variables'], {'user_id': '42', 'offset': 0, 'count': 100});
        return http.Response(
            jsonEncode({
              'data': {
                'ascentsForUser': [
                  {'id': 'a1', 'date': '2026-09-14'},
                ]
              }
            }),
            200);
      }),
    );
    final page = await api.ascentsPage(token: 't1', userId: '42', offset: 0);
    expect(page, hasLength(1));
    expect(page.first['id'], 'a1');
  });

  test('ascentsPage honors Retry-After on 429 then succeeds', () async {
    var calls = 0;
    final api = KayaApi(
      client: MockClient((_) async {
        calls++;
        if (calls == 1) {
          return http.Response('rate limited', 429,
              headers: {'retry-after': '0'});
        }
        return http.Response(
            jsonEncode({'data': {'ascentsForUser': []}}), 200);
      }),
    );
    final page = await api.ascentsPage(token: 't', userId: 'u', offset: 0);
    expect(page, isEmpty);
    expect(calls, 2);
  });

  test('ascentsPage throws KayaAuthException on 401', () async {
    final api =
        KayaApi(client: MockClient((_) async => http.Response('no', 401)));
    expect(() => api.ascentsPage(token: 'x', userId: 'u', offset: 0),
        throwsA(isA<KayaAuthException>()));
  });

  test('ascentsPage surfaces GraphQL errors', () async {
    final api = KayaApi(
        client: MockClient((_) async => http.Response(
            jsonEncode({'errors': [{'message': 'RATE_LIMITED'}]}), 200)));
    expect(() => api.ascentsPage(token: 't', userId: 'u', offset: 0),
        throwsA(isA<StateError>()));
  });
}
```

- [ ] **Step 2: Run to verify RED**

Run: `cd ~/repos/ledger && flutter test test/kaya_api_test.dart 2>&1 | tail -5`
Expected: compile failure — `kaya_api.dart` does not exist.

- [ ] **Step 3: Implement `lib/services/integrations/kaya_api.dart`**

```dart
/// Thin client for Kaya's UNOFFICIAL API (reverse-engineered; no public
/// program exists — see the design spec's ToS note). Login is plain
/// email/password → bearer + refresh token; data comes from the same
/// GraphQL endpoint the web app uses. The server 403s requests without
/// browser-looking Origin/Referer headers.
library;

import 'dart:convert';

import 'package:http/http.dart' as http;

const _kBase = 'https://kaya-beta.kayaclimb.com';
const _kOrigin = 'https://kaya-app.kayaclimb.com';

/// Bearer token was rejected — caller should refresh and retry.
class KayaAuthException implements Exception {}

class KayaAuth {
  final String token;
  final String refreshToken;
  final String userId;
  KayaAuth({required this.token, required this.refreshToken, required this.userId});
}

/// Verified against Asherlc/dofek's observed-API contract (Task 5);
/// adjust there first if Kaya drifts.
const kAscentsQuery = r'''
query AscentsForUser($user_id: ID!, $offset: Int, $count: Int) {
  ascentsForUser(user_id: $user_id, offset: $offset, count: $count) {
    id
    date
    comment
    attempts
    ascent_type { name }
    gym { name }
    destination { name }
    climb {
      name
      lead
      climb_type { name }
      grade { name }
      gym { name }
    }
  }
}
''';

class KayaApi {
  KayaApi({http.Client? client}) : _client = client ?? http.Client();
  final http.Client _client;

  Map<String, String> get _headers => {
        'Content-Type': 'application/json',
        'Origin': _kOrigin,
        'Referer': '$_kOrigin/',
      };

  Future<KayaAuth> login(String email, String password) async {
    final resp = await _client.post(
      Uri.parse('$_kBase/api/user/login'),
      headers: _headers,
      body: jsonEncode({'email': email, 'password': password}),
    );
    if (resp.statusCode != 200) {
      throw StateError('Kaya login failed (${resp.statusCode})');
    }
    final body = jsonDecode(resp.body) as Map<String, dynamic>;
    final token = body['token'] as String?;
    final refresh = body['refresh_token'] as String?;
    final userId = ((body['user'] as Map?)?['id'])?.toString();
    if (token == null || refresh == null || userId == null) {
      throw StateError('Kaya login: unexpected response shape');
    }
    return KayaAuth(token: token, refreshToken: refresh, userId: userId);
  }

  Future<String> refresh(String refreshToken) async {
    final resp = await _client.post(
      Uri.parse('$_kBase/api/user/refresh-token'),
      headers: _headers,
      body: jsonEncode({'refresh_token': refreshToken}),
    );
    if (resp.statusCode != 200) {
      throw StateError('Kaya token refresh failed (${resp.statusCode})');
    }
    final token = (jsonDecode(resp.body) as Map<String, dynamic>)['token'];
    if (token is! String) throw StateError('Kaya refresh: no token in response');
    return token;
  }

  /// One page of ascents. Retries 429s (Retry-After honored, expo
  /// fallback) up to 3 times; 401 → [KayaAuthException].
  Future<List<Map<String, dynamic>>> ascentsPage({
    required String token,
    required String userId,
    required int offset,
    int count = 100,
  }) async {
    for (var attempt = 0;; attempt++) {
      final resp = await _client.post(
        Uri.parse('$_kBase/graphql'),
        headers: {..._headers, 'Authorization': 'Bearer $token'},
        body: jsonEncode({
          'query': kAscentsQuery,
          'variables': {'user_id': userId, 'offset': offset, 'count': count},
        }),
      );
      if (resp.statusCode == 429 && attempt < 3) {
        final wait =
            int.tryParse(resp.headers['retry-after'] ?? '') ?? (5 << attempt);
        await Future.delayed(Duration(seconds: wait));
        continue;
      }
      if (resp.statusCode == 401) throw KayaAuthException();
      if (resp.statusCode != 200) {
        throw StateError('Kaya graphql failed (${resp.statusCode})');
      }
      final decoded = jsonDecode(resp.body) as Map<String, dynamic>;
      final errors = decoded['errors'];
      if (errors is List && errors.isNotEmpty) {
        throw StateError('Kaya graphql error: ${jsonEncode(errors.first)}');
      }
      final list = (decoded['data'] as Map?)?['ascentsForUser'];
      return [
        for (final a in (list as List? ?? const []))
          if (a is Map) a.cast<String, dynamic>(),
      ];
    }
  }
}
```

- [ ] **Step 4: Run to verify GREEN**

Run: `cd ~/repos/ledger && flutter test test/kaya_api_test.dart 2>&1 | tail -3`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cd ~/repos/ledger
git add lib/services/integrations/kaya_api.dart test/kaya_api_test.dart
git commit -m "feat(kaya): HTTP client for Kaya's unofficial API

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: App — transform + reconcile-diff functions

**Files:**
- Create: `~/repos/ledger/lib/services/integrations/kaya.dart` (top-level functions only in this task; the class comes in Task 8)
- Test: `~/repos/ledger/test/kaya_transform_test.dart`

- [ ] **Step 1: Write the failing tests**

```dart
import 'package:flutter_test/flutter_test.dart';

import 'package:airledger/services/integrations/kaya.dart';

Map<String, dynamic> _gymAscent() => {
      'id': 'a1',
      'date': '2026-09-14T19:03:00.000Z',
      'comment': 'felt easy',
      'attempts': 2,
      'ascent_type': {'name': 'Flash'},
      'gym': {'name': 'Movement Sunnyvale'},
      'destination': null,
      'climb': {
        'name': 'Pink Crimps',
        'lead': null,
        'climb_type': {'name': 'Boulder'},
        'grade': {'name': 'V5'},
        'gym': {'name': 'Movement Sunnyvale'},
      },
    };

void main() {
  test('gym boulder ascent → record', () {
    final recs = kayaAscentsToRecords([_gymAscent()]);
    expect(recs, hasLength(1));
    final r = recs.first;
    expect(r['kaya_id'], {'kind': 'string', 'value': 'a1'});
    expect(r['date'], {'kind': 'date', 'value': '2026-09-14'});
    expect(r['climb_name'], {'kind': 'string', 'value': 'Pink Crimps'});
    expect(r['climb_type'], {'kind': 'string', 'value': 'boulder'});
    expect(r['grade'], {'kind': 'string', 'value': 'V5'});
    expect(r['ascent_type'], {'kind': 'string', 'value': 'flash'});
    expect(r['attempts'], {'kind': 'int', 'value': 2});
    expect(r['gym'], {'kind': 'string', 'value': 'Movement Sunnyvale'});
    expect(r.containsKey('location'), isFalse);
    expect(r['notes'], {'kind': 'string', 'value': 'felt easy'});
    expect(r.containsKey('lead'), isFalse, reason: 'lead null for boulders');
  });

  test('outdoor route ascent uses location and lead', () {
    final a = _gymAscent()
      ..['id'] = 'a2'
      ..['gym'] = null
      ..['destination'] = {'name': 'Castle Rock'}
      ..['climb'] = {
        'name': 'The Great Roof',
        'lead': true,
        'climb_type': {'name': 'Sport'},
        'grade': {'name': '5.12a'},
        'gym': null,
      };
    final r = kayaAscentsToRecords([a]).first;
    expect(r['climb_type'], {'kind': 'string', 'value': 'route'});
    expect(r['lead'], {'kind': 'bool', 'value': true});
    expect(r['location'], {'kind': 'string', 'value': 'Castle Rock'});
    expect(r.containsKey('gym'), isFalse);
  });

  test('tolerates epoch-ms dates and skips junk', () {
    final epoch = _gymAscent()..['date'] = 1789500000000; // ms
    final noId = _gymAscent()..remove('id');
    final noDate = _gymAscent()..remove('date');
    final recs = kayaAscentsToRecords([epoch, noId, noDate, 'garbage']);
    expect(recs, hasLength(1));
    expect((recs.first['date'] as Map)['value'], matches(r'^\d{4}-\d{2}-\d{2}$'));
  });

  test('kayaDay parses ISO, epoch seconds, and JS-style strings', () {
    expect(kayaDay('2026-09-14'), '2026-09-14');
    expect(kayaDay('2026-09-14T19:03:00.000Z'), '2026-09-14');
    expect(kayaDay(1757905380), isNotNull); // epoch seconds
    expect(kayaDay('Sun May 23 2021 14:15:39 GMT+0000 (GMT)'), '2021-05-23');
    expect(kayaDay(null), isNull);
    expect(kayaDay('not a date'), isNull);
  });

  test('kayaDeletedIds diffs known against fetched', () {
    expect(
      kayaDeletedIds(fetchedIds: {'a1', 'a3'}, knownIds: {'a1', 'a2', 'a3', 'a0'}),
      ['a0', 'a2'],
    );
    expect(kayaDeletedIds(fetchedIds: {'a1'}, knownIds: {'a1'}), isEmpty);
  });
}
```

- [ ] **Step 2: Run to verify RED**

Run: `cd ~/repos/ledger && flutter test test/kaya_transform_test.dart 2>&1 | tail -5`
Expected: compile failure — `kaya.dart` does not exist.

- [ ] **Step 3: Implement the functions in `lib/services/integrations/kaya.dart`**

```dart
/// Kaya → climbing integration.
///
/// Email/password login against Kaya's unofficial API (kaya_api.dart),
/// full-logbook walk every pull (no cursor: the API's sort order is
/// unverified and the logbook is a handful of pages — correctness never
/// depends on order because ingest upserts by kaya_id), reconcile =
/// known ids minus fetched ids → deleted_ids.
library;

import 'dart:convert';

import 'package:airledger_engine/airledger_engine.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'integration.dart';
import 'kaya_api.dart';

/// Transform Kaya GraphQL ascents into engine ingest records — one per
/// ascent, keyed by kaya_id. Malformed entries are dropped.
List<Map<String, dynamic>> kayaAscentsToRecords(List<dynamic> ascents) {
  final out = <Map<String, dynamic>>[];
  for (final a in ascents) {
    if (a is! Map) continue;
    final id = a['id']?.toString() ?? '';
    final day = kayaDay(a['date']);
    if (id.isEmpty || day == null) continue;
    final climb = (a['climb'] as Map?) ?? const {};
    final gym = _name(a['gym']) ?? _name(climb['gym']);
    final location = _name(a['destination']);
    final climbType = _name(climb['climb_type'])?.toLowerCase();
    final lead = climb['lead'];
    out.add({
      'kaya_id': {'kind': 'string', 'value': id},
      'date': {'kind': 'date', 'value': day},
      if (climb['name'] is String)
        'climb_name': {'kind': 'string', 'value': climb['name']},
      if (climbType != null)
        'climb_type': {
          'kind': 'string',
          // Kaya climb types beyond Boulder (Sport/Trad/Top Rope…) all
          // collapse to 'route' for the tracker's two-way split.
          'value': climbType == 'boulder' ? 'boulder' : 'route',
        },
      if (_name(climb['grade']) != null)
        'grade': {'kind': 'string', 'value': _name(climb['grade'])},
      if (_name(a['ascent_type']) != null)
        'ascent_type': {
          'kind': 'string',
          'value': _name(a['ascent_type'])!.toLowerCase(),
        },
      if (a['attempts'] is num)
        'attempts': {'kind': 'int', 'value': (a['attempts'] as num).toInt()},
      if (lead is bool) 'lead': {'kind': 'bool', 'value': lead},
      if (gym != null) 'gym': {'kind': 'string', 'value': gym},
      if (gym == null && location != null)
        'location': {'kind': 'string', 'value': location},
      if (a['comment'] is String && (a['comment'] as String).isNotEmpty)
        'notes': {'kind': 'string', 'value': a['comment']},
    });
  }
  return out;
}

String? _name(dynamic node) =>
    node is Map && node['name'] is String ? node['name'] as String : null;

/// Kaya's date wire format is not pinned down (ISO observed; the old CSV
/// export used JS toString; epoch is cheap to accept) — parse all three.
String? kayaDay(dynamic v) {
  if (v is num) {
    final ms = v > 1000000000000 ? v.toInt() : v.toInt() * 1000;
    return _isoDate(DateTime.fromMillisecondsSinceEpoch(ms));
  }
  if (v is! String || v.isEmpty) return null;
  final parsed = DateTime.tryParse(v);
  if (parsed != null) return _isoDate(parsed);
  // JS style: "Sun May 23 2021 14:15:39 GMT+0000 (GMT)"
  const months = {
    'Jan': 1, 'Feb': 2, 'Mar': 3, 'Apr': 4, 'May': 5, 'Jun': 6,
    'Jul': 7, 'Aug': 8, 'Sep': 9, 'Oct': 10, 'Nov': 11, 'Dec': 12,
  };
  final parts = v.split(' ');
  if (parts.length >= 4) {
    final m = months[parts[1]];
    final d = int.tryParse(parts[2]);
    final y = int.tryParse(parts[3]);
    if (m != null && d != null && y != null) {
      return _isoDate(DateTime(y, m, d));
    }
  }
  return null;
}

/// Ascent ids the ledger credits to Kaya that Kaya no longer returns —
/// the `deleted_ids` for the reconcile.
List<String> kayaDeletedIds({
  required Set<String> fetchedIds,
  required Set<String> knownIds,
}) =>
    (knownIds.difference(fetchedIds).toList()..sort());

String _isoDate(DateTime d) => '${d.year.toString().padLeft(4, '0')}-'
    '${d.month.toString().padLeft(2, '0')}-'
    '${d.day.toString().padLeft(2, '0')}';
```

- [ ] **Step 4: Run to verify GREEN**

Run: `cd ~/repos/ledger && flutter test test/kaya_transform_test.dart 2>&1 | tail -3`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
cd ~/repos/ledger
git add lib/services/integrations/kaya.dart test/kaya_transform_test.dart
git commit -m "feat(kaya): ascent transform + reconcile diff

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: App — `KayaIntegration` class

**Files:**
- Modify: `~/repos/ledger/lib/services/integrations/kaya.dart` (append the class)

No new unit tests: pull orchestration mirrors WithingsIntegration, which the repo deliberately keeps as thin untested glue over the tested pure pieces (transform, diff, client). On-device verification is Task 10.

- [ ] **Step 1: Append the class to `kaya.dart`**

```dart
const _kMinPullInterval = Duration(hours: 6);
const _kPageDelay = Duration(seconds: 2);

class KayaIntegration implements Integration {
  KayaIntegration({
    required this.repo,
    required this.climbingViewJson,
    KayaApi? api,
  }) : api = api ?? KayaApi();

  final EngineLedgerRepository repo;
  final Map<String, dynamic> climbingViewJson;
  final KayaApi api;

  static const _storage = FlutterSecureStorage();
  static const _kToken = 'kaya_token';
  static const _kRefresh = 'kaya_refresh';
  static const _kUserId = 'kaya_user_id';

  static const _kLastPull = 'integration_kaya_last_pull';
  static const _kStatus = 'integration_kaya_status';
  static const _kError = 'integration_kaya_error';
  static const _kIds = 'integration_kaya_ids';

  @override
  String get id => 'kaya';
  @override
  String get displayName => 'Kaya';
  @override
  String get targetDescription => '→ climbing';
  @override
  bool get isConfigured => true; // user credentials only; no app secrets

  @override
  Future<bool> get isConnected async =>
      (await _storage.read(key: _kRefresh)) != null;

  @override
  Future<String> get statusLine async {
    if (!await isConnected) return 'Not connected';
    final status = await repo.metaGet(_kStatus);
    if (status == 'reconnect') return 'Reconnect needed';
    if (status == 'error') {
      final e = await repo.metaGet(_kError) ?? 'unknown';
      return 'Error: $e';
    }
    final last = await repo.metaGet(_kLastPull);
    final count = _decodeIds(await repo.metaGet(_kIds)).length;
    final when = last == null
        ? 'never'
        : DateTime.tryParse(last)?.toLocal().toString().substring(11, 16) ??
            last;
    return 'Connected · last pulled $when · $count ascent(s) synced';
  }

  @override
  Map<String, Future<void> Function(BuildContext)> get extraMenuActions =>
      const {};

  @override
  Future<void> connect(BuildContext context) async {
    final email = TextEditingController();
    final password = TextEditingController();
    String? error;
    var busy = false;
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => StatefulBuilder(
        builder: (ctx, setState) => AlertDialog(
          title: const Text('Connect Kaya'),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              TextField(
                controller: email,
                keyboardType: TextInputType.emailAddress,
                decoration: const InputDecoration(labelText: 'Email'),
              ),
              TextField(
                controller: password,
                obscureText: true,
                decoration: const InputDecoration(labelText: 'Password'),
              ),
              if (error != null)
                Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: Text(error!,
                      style: TextStyle(color: Theme.of(ctx).colorScheme.error)),
                ),
            ],
          ),
          actions: [
            TextButton(
              onPressed: busy ? null : () => Navigator.of(ctx).pop(false),
              child: const Text('Cancel'),
            ),
            FilledButton(
              onPressed: busy
                  ? null
                  : () async {
                      setState(() => busy = true);
                      try {
                        final auth =
                            await api.login(email.text.trim(), password.text);
                        await _storage.write(key: _kToken, value: auth.token);
                        await _storage.write(
                            key: _kRefresh, value: auth.refreshToken);
                        await _storage.write(key: _kUserId, value: auth.userId);
                        if (ctx.mounted) Navigator.of(ctx).pop(true);
                      } catch (e) {
                        setState(() {
                          busy = false;
                          error = '$e';
                        });
                      }
                    },
              child: const Text('Connect'),
            ),
          ],
        ),
      ),
    );
    if (ok == true) {
      await repo.metaSet(_kStatus, 'ok');
      // First pull = full backfill; don't block the UI on it.
      // ignore: unawaited_futures
      pull(force: true);
    }
  }

  @override
  Future<void> disconnect() async {
    await _storage.delete(key: _kToken);
    await _storage.delete(key: _kRefresh);
    await _storage.delete(key: _kUserId);
    await repo.metaSet(_kStatus, '');
    await repo.metaSet(_kError, '');
    // _kIds intentionally kept: reconnect stays consistent with the
    // provenance the engine still holds.
  }

  /// Every pull is a full walk + full reconcile (see library docs), so
  /// [fullReconcile] adds nothing beyond what a normal pull does.
  @override
  Future<void> pull({bool force = false, bool fullReconcile = false}) async {
    if (!await isConnected) return;
    try {
      if (!force) {
        final last = await repo.metaGet(_kLastPull);
        final lastAt = last == null ? null : DateTime.tryParse(last);
        if (lastAt != null &&
            DateTime.now().difference(lastAt) < _kMinPullInterval) {
          return;
        }
      }
      var token = await _storage.read(key: _kToken) ?? '';
      final userId = await _storage.read(key: _kUserId) ?? '';
      var refreshed = false;
      final ascents = <Map<String, dynamic>>[];
      var offset = 0;
      while (true) {
        List<Map<String, dynamic>> page;
        try {
          page = await api.ascentsPage(
              token: token, userId: userId, offset: offset);
        } on KayaAuthException {
          if (refreshed) {
            await repo.metaSet(_kStatus, 'reconnect');
            return;
          }
          refreshed = true;
          final t = await _refreshToken();
          if (t == null) return; // reconnect status already set
          token = t;
          continue;
        }
        ascents.addAll(page);
        if (page.length < 100 || offset >= 20000) break; // 20k = runaway guard
        offset += 100;
        await Future.delayed(_kPageDelay);
      }

      final records = kayaAscentsToRecords(ascents);
      final fetchedIds = {
        for (final r in records) ((r['kaya_id'] as Map)['value']) as String,
      };
      final known = _decodeIds(await repo.metaGet(_kIds));
      final deleted = kayaDeletedIds(fetchedIds: fetchedIds, knownIds: known);
      if (records.isNotEmpty || deleted.isNotEmpty) {
        await repo.ingest(climbingViewJson, {
          'source': 'kaya',
          'match_field': 'kaya_id',
          'owned_fields': [
            'kaya_id', 'date', 'climb_name', 'climb_type', 'grade',
            'ascent_type', 'attempts', 'lead', 'gym', 'location',
          ],
          'fill_if_blank_fields': ['notes'],
          'records': records,
          'deleted_ids': deleted,
        });
        await repo.metaSet(_kIds, jsonEncode(fetchedIds.toList()..sort()));
      }
      await repo.metaSet(_kLastPull, DateTime.now().toIso8601String());
      await repo.metaSet(_kStatus, 'ok');
      await repo.metaSet(_kError, '');
    } catch (e) {
      await repo.metaSet(_kStatus, 'error');
      await repo.metaSet(_kError, e.toString());
    }
  }

  Set<String> _decodeIds(String? json) {
    if (json == null || json.isEmpty) return <String>{};
    final decoded = jsonDecode(json);
    return decoded is List ? decoded.cast<String>().toSet() : <String>{};
  }

  Future<String?> _refreshToken() async {
    final refresh = await _storage.read(key: _kRefresh);
    if (refresh == null) return null;
    try {
      final token = await api.refresh(refresh);
      await _storage.write(key: _kToken, value: token);
      return token;
    } catch (_) {
      await repo.metaSet(_kStatus, 'reconnect');
      return null;
    }
  }
}
```

- [ ] **Step 2: Analyze + full test suite**

Run: `cd ~/repos/ledger && flutter analyze 2>&1 | tail -3 && flutter test 2>&1 | tail -8`
Expected: analyze at the ~32-info baseline; tests at exactly the 7 known failures.

- [ ] **Step 3: Commit**

```bash
cd ~/repos/ledger
git add lib/services/integrations/kaya.dart
git commit -m "feat(kaya): KayaIntegration — connect dialog + full-walk pull/reconcile

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 9: App — register the integration

**Files:**
- Modify: `~/repos/ledger/lib/ui/home_screen.dart` (the `_initialize()` block that finds `weightView` and calls `IntegrationRegistry.init`, currently ~lines 138–157)

- [ ] **Step 1: Find the climbing view alongside the weight view**

In `_initialize()`, extend the existing view-lookup loop:

```dart
      ViewSchema? weightView;
      ViewSchema? climbingView;
      for (final v in views) {
        if (v.name == 'weight') weightView = v;
        if (v.name == 'climbing') climbingView = v;
      }
```

- [ ] **Step 2: Register KayaIntegration**

In the `IntegrationRegistry.init(integrations: [...])` list, after the Withings entry:

```dart
        if (climbingView != null)
          KayaIntegration(
            repo: repo.repo,
            climbingViewJson: viewSchemaToEngineJson(climbingView),
          ),
```

Add the import at the top of home_screen.dart alongside the withings one:

```dart
import '../services/integrations/kaya.dart';
```

- [ ] **Step 3: Analyze + tests**

Run: `cd ~/repos/ledger && flutter analyze 2>&1 | tail -3 && flutter test 2>&1 | tail -8`
Expected: baselines hold (32 infos / 7 known failures).

- [ ] **Step 4: Commit**

```bash
cd ~/repos/ledger
git add lib/ui/home_screen.dart
git commit -m "feat(kaya): register Kaya integration when the climbing view exists

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 10: Build, deploy, verify on device

- [ ] **Step 1: Confirm the dylib is fresh (Task 3 already ran; re-verify)**

Run: `strings $(find ~/repos/ledger -name libairledger_engine.so | head -1) | grep -c match_field`
Expected: ≥1. A zero here means the APK would silently drop `match_field` — stop and redo Task 3.

- [ ] **Step 2: Build + install + launch**

Run: `cd ~/repos/ledger && dart run tool/brand.dart --config ~/repos/airledger-fitness/ledger.yaml`
Expected: schemas (including climbing) synced to assets, APK built and installed on `66260DLKX00010`, app launches. If the device is unplugged: `adb -s 66260DLKX00010 wait-for-device` first.

- [ ] **Step 3: On-device verification checklist (user assists)**

1. Home list shows a **climbing** tracker (schema loaded).
2. Integrations screen shows a **Kaya → climbing** card with Connect.
3. Connect with real credentials → dialog closes, status flips to Connected.
4. Full backfill lands: climbing timeline populates with the Kaya logbook; ascent count in the card status matches expectations.
5. `climbing` tab appears in the Sheets workbook with the rows after the next sync.
6. Tap "Sync now" a second time → status updates, ascent count stable, no duplicate rows (match_field replay is a no-op).
7. Edit one row's notes in the app, "Sync now" again → the note survives (fill-if-blank).

If step 3–4 fail with a GraphQL/shape error in the card status: capture the error string, revisit Task 5's contract (`adb logcat | grep -i kaya` helps), fix the query/transform, and rerun from Task 6's tests.

- [ ] **Step 4: Update docs**

- `~/repos/ledger/CLAUDE.md`: add Kaya to the current-feature-state list (one bullet: unofficial API, full-walk pulls, match_field ingest) and remove nothing.
- `~/repos/airledger/docs/integrations.md`: add a short Kaya section (background-pull pattern, match_field mode).

```bash
cd ~/repos/ledger && git add CLAUDE.md && git commit -m "docs: Kaya integration in feature state

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
cd ~/repos/airledger && git add docs/integrations.md && git commit -m "docs: match_field ingest mode + Kaya integration

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-review notes

- **Spec coverage:** engine match_field (§1 → Tasks 1–2), dylib trap (§1 → Task 3), schemas + push trap (§2 → Task 4), API client/auth/pull (§3 → Tasks 5–8), error handling (§4 → Tasks 6 & 8 code), testing (§5 → Tasks 1,2,6,7 + baselines), deploy order (§6 → Tasks 3,4,10 sequence). Sort-order risk resolved via the spec's sanctioned fallback: full walk every pull, no cursor.
- **Deviation from spec, intentional:** no 90-day reconcile window and no cursor meta — the full walk makes the window redundant (every pull reconciles all history). `integration_kaya_ids` replaces the cursor as the deletion baseline.
- **Type consistency:** `KayaAuth{token, refreshToken, userId}`, `kayaAscentsToRecords`, `kayaDay`, `kayaDeletedIds`, `KayaAuthException`, `kAscentsQuery` are each defined once (Tasks 6–7) and consumed with those exact names (Task 8).
