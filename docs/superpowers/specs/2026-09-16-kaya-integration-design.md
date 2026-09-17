# Kaya → climbing integration

Pull the user's full Kaya (kayaclimb.com) logbook — per-ascent rows,
boulders + routes, gym + outdoor — into a new `climbing` view via the
background-pull integration pattern (Withings precedent). Spans three
repos: engine (`airledger`), app (`ledger`), schemas
(`airledger-fitness`).

Approved 2026-09-16. Decisions made during brainstorming:
per-ascent granularity (not daily summaries) · unofficial GraphQL API
(not CSV export) · everything (not boulders-only) · full-history
backfill · `rating`/`stiffness` excluded for now · grades stored as
Kaya's strings, not normalized.

## 1. Engine: `match_field` ingest extension

Today `ingest()` (src/store/ingest.rs) matches records to rows by the
view's `date_field` — one row per day. Per-ascent data needs many rows
per day, so `IngestBatch` gains two optional fields:

```json
{
  "source": "kaya",
  "match_field": "kaya_id",          // NEW: match rows by this dimension
  "deleted_ids": ["a1", "a2"],       // NEW: unwind by match_field value
  "owned_fields": [...],
  "fill_if_blank_fields": [...],
  "records": [...],
  "deleted_dates": []                 // still valid; unused by Kaya
}
```

Semantics:

- `match_field` set → existing rows index by that dimension's display
  value; records upsert by that key. Records whose `match_field` value
  matches no row create a new row. Rows without a value for
  `match_field` (hand-entered) are invisible to the batch — never
  matched, never touched.
- Records must still carry `date_field` values (rows live on the
  timeline); uniqueness constraints on date no longer apply when
  `match_field` is set.
- `deleted_ids` follows `deleted_dates` provenance rules exactly:
  source created the row and all source-written fields are unedited →
  delete the row (tombstone, sheet row removed on sync); user edited
  anything → clear only the source-written fields.
- `match_field` absent → behavior identical to today. Withings is
  untouched.
- Duplicate `match_field` values inside one batch: last record wins
  (dedup before apply); duplicate values across existing rows (shouldn't
  happen, but sheets are editable): first match wins, log a warning.

Tests: Rust unit tests for upsert/create/no-touch-manual-rows,
`deleted_ids` unwind both branches, batch JSON round-trip. No FFI
signature change (batch is JSON through the existing call).

**Deploy trap #1 applies**: rebuild the dylib
(`sdk-dart/scripts/build-android.sh`) and sanity-check
`strings .../libairledger_engine.so | grep match_field` before the APK
build.

## 2. Schemas: `climbing` view (airledger-fitness)

New `climbing.view.yml` + `climbing.input.yml`; new `climbing` tab in
the main workbook (created by ensureTable/sync). `date_field: date`.

Dimensions:

| name | type | notes |
|---|---|---|
| id | string | engine row id |
| kaya_id | string | Kaya ascent id; match key; blank for manual rows |
| date | date | ascent date |
| climb_name | string | |
| climb_type | string | boulder / route |
| grade | string | Kaya's string verbatim (V5, 5.12a) |
| ascent_type | string | flash / redpoint / onsight / send |
| attempts | number | |
| lead | boolean | routes only |
| gym | string | blank for outdoor |
| location | string | outdoor destination/area; blank for gym |
| notes | string | Kaya comment; user-annotatable |

Renders as a normal tracker (input overlay present): rows editable,
manual adds allowed (no kaya_id → ingest never touches them). The
coach's 28-day ledger dump includes it automatically.

**Deploy trap #2 applies**: push to airledger-fitness or SchemaSync
reverts the view on device within ~5 minutes.

## 3. App: `KayaIntegration` (ledger)

`lib/services/integrations/kaya.dart`, implementing the existing
`Integration` interface; registered in `home_screen.dart` next to
Withings; card appears on the Integrations screen automatically;
`climbing` flows to Sheets through the existing scheduler (it's a
gsheets entry view).

**API surface** (unofficial, reverse-engineered; three independent
public projects agree on it):

- `POST https://kaya-beta.kayaclimb.com/api/user/login`
  `{email, password}` → `{token, refresh_token, user: {id}}`
- `POST /api/user/refresh-token` `{refresh_token}` → `{token}`
- `POST /graphql` with `Authorization: Bearer <token>`;
  query `ascentsForUser(user_id, offset, count)` → id, session_id,
  date, comment, attempts, ascent_type{name}, gym{name},
  climb{name, lead, climb_type{name}, grade{name}}, destination/area
  fields for outdoor.
- All requests send `Origin: https://kaya-app.kayaclimb.com` and
  matching `Referer` (server 403s without them).

**Connect**: in-app dialog with email + password fields (no OAuth
exists). Login → store bearer token, refresh token, user id in
`FlutterSecureStorage`. The password is never persisted. Refresh-token
failure → status meta prompts "Reconnect needed"; user re-enters
credentials.

**Pull** (never throws; errors → `integration_kaya_error` meta):

1. Refresh the bearer token if expired.
2. Page `ascentsForUser`, 100/page, ~2 s between pages, honor
   `Retry-After` on 429. Assumed newest-first; the actual sort order is
   unverified — confirm against the live API during implementation and,
   if it can't be relied on, drop the cursor early-exit and treat every
   pull as a full walk (correctness never depends on order: ingest
   upserts are idempotent).
3. First pull after connect: walk the entire logbook (full backfill).
   Routine pulls: walk until a page is entirely at-or-older than the
   stored cursor (newest ascent id + date already seen).
4. Transform → kind-tagged records (`kaya_id`, `date`, fields above).
   gym vs outdoor: `gym` present → gym fields; else destination/area →
   `location`.
5. `repo.ingest(climbingViewJson, batch)` with `source: 'kaya'`,
   `match_field: 'kaya_id'`, `owned_fields`: everything except notes,
   `fill_if_blank_fields: ['notes']` (user annotations survive).
6. Reconcile: re-page the last 90 days, diff `kaya_id`s present locally
   (credited to kaya in provenance) vs returned → `deleted_ids`.
   `fullReconcile` widens to all history.
7. Update cursor + `integration_kaya_last_pull` meta.

**Config**: no app-level secrets (user credentials only), so no
`config.yaml` block; `isConfigured` is always true.

**Cadence**: same min-interval convention as Withings via `pullDue()`.

## 4. Error handling

- `pull()` never throws; all failures land in status meta and the card.
- 429 → exponential backoff honoring `Retry-After`; give up the tick
  gracefully (next scheduled sync retries).
- Auth failures distinguish "token expired, refresh worked" (silent)
  from "refresh rejected" (Reconnect needed).
- API drift (it's unofficial): GraphQL errors or shape mismatches →
  clear error string in the card status, no partial ingest (transform
  the full page set before ingesting each page's batch).

## 5. Testing

- Rust: ingest unit tests + round-trip (section 1).
- Dart: transform tests (GraphQL JSON fixtures → records, gym vs
  outdoor, boulder vs route, lead flag); pagination/cursor walk and
  429/Retry-After handling against a fake HTTP client; login/refresh
  storage flow. TDD throughout.
- Existing suite baselines: 7 known failures (3 live-DB integration,
  4 schema_loader); anything else is a regression.

## 6. Deploy order

1. Engine: implement + test → `build-android.sh` → strings check.
2. Schemas: add view files → push airledger-fitness.
3. App: integration + registration → `brand.dart` build → device.
4. Verify: connect with real credentials, watch full backfill, confirm
   rows in timeline + `climbing` sheet tab, then a second pull is
   incremental (cursor respected).

## Risks / known trade-offs

- **ToS**: Kaya's terms prohibit scraping; this is personal-use access
  to the user's own data, accepted knowingly during design.
- **API drift**: unofficial endpoint can change without notice;
  mitigation is the error-surfacing in section 4, not prevention.
- **Rate limits**: observed 429s with Retry-After; pull spacing + page
  size chosen to match community-observed safe behavior.
- Session-level data (start/end time, session notes) is deliberately
  out of scope — ascents only. A `sessions`-derived daily summary can
  be added later without engine changes if wanted.
