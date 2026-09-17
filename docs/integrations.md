# Integrations: patterns for external data sources

Two patterns exist. Pick by asking: *is the user present when the data
arrives?*

| Pattern | Example | Data path |
|---------|---------|-----------|
| Background pull | Withings → weight | API → transform → `ingest()` (provenance, unwind) |
| Live/interactive | Whoop BLE heart rate | device stream → form widget → normal form save |

Both register on the app's integrations screen via the same interface.

## The `Integration` interface (app)

`~/repos/ledger/lib/services/integrations/integration.dart`:
`id`, `displayName`, `targetDescription`, `isConfigured`, `isConnected`,
`statusLine`, `connect(context)`, `disconnect()`, `pull({force,
fullReconcile})`. Contract notes:

- `pull()` **must never throw** — failures land in status meta and retry
  on the next trigger. Live-only sources implement it as a no-op.
- All state the card displays comes from ledger meta under
  `integration_<id>_*` keys (last pull, cursor, status, device id…), so
  the UI and engine share one source of truth.
- Instances register in `registry.dart`; `sync_scheduler.dart` calls
  `IntegrationRegistry.pullDue()` before each sync.

## Background pull pattern (reference: Withings)

`~/repos/ledger/lib/services/integrations/withings.dart`:

1. `connect()` runs OAuth2 in a WebView; tokens go to
   `flutter_secure_storage` (never ledger meta).
2. `pull()` honors a per-source minimum interval (Withings: 6h) tracked in
   `integration_withings_last_pull`; `force` bypasses.
3. Fetch from the API, transform to one record per date
   (`withingsGroupsToRecords`), collect deleted dates for the reconcile
   window (`fullReconcile` sweeps all history).
4. Call `EngineLedgerRepository.ingest()` with
   `owned_fields`/`fill_if_blank_fields`; the engine handles merge,
   provenance, and deletion unwind — see
   [local-first-store.md](local-first-store.md).
5. Write cursor + human status back to meta; sync scheduler pushes the
   ingested rows to Sheets on its normal cycle.

## Row-grained pull pattern (reference: Kaya)

`~/repos/ledger/lib/services/integrations/kaya.dart` (+ `kaya_api.dart`).
Same shape as Withings, but the unit is one row per *ascent*, not per
day, so ingest runs in **match_field mode**: the batch sets
`match_field: kaya_id` and rows upsert by that dimension's value
(`src/store/ingest.rs`); many rows per day are fine. Deletions arrive as
`deleted_ids` (values of the match field) instead of `deleted_dates`,
with the same provenance rules; both identity fields (`date_field` +
`match_field`) are exempt from the clear branch. Rows without a match
value — hand-entered ones — are invisible to the batch. An unknown
`match_field` errors loudly.

Kaya-specific choices worth copying:

- **No cursor.** The API's sort order is unverified, so every pull walks
  the full paginated list and reconciles all history by diffing raw ids
  against the `integration_kaya_ids` baseline. Correctness never depends
  on order; upserts are idempotent.
- **Deletion safety, three layers**: GraphQL shape drift (missing/null
  data field) throws instead of reading as an empty list; the diff keys
  on ids from the RAW wire response, never on transform output; an empty
  fetch against a non-empty baseline refuses to diff unless the user
  explicitly runs Full reconcile.
- **Exclusive-pair fields ship explicit nulls** (`{"kind":"null"}` for
  gym/location and lead): the engine never clears an owned field that is
  absent from the record, so cross-boundary revisions must send the null.
  Fields that can go absent through wire drift stay omit-don't-clear.
- Auth is plain email/password → bearer + refresh token (no OAuth); the
  API is unofficial and reverse-engineered, so requests spoof the web
  app's Origin/Referer and the client treats any shape surprise as an
  error, not data.

## Live/interactive pattern (reference: Whoop HR)

Spec: `docs/superpowers/specs/2026-09-11-whoop-live-hr-design.md`.

Whoop's API has no continuous HR; the band broadcasts the standard BLE
Heart Rate Service (0x180D, measurement characteristic 0x2A37) when "HR
Broadcast" is on in the Whoop app. So the integration is generic BLE HR:

- A `HeartRateService` in the app (flutter_blue_plus) scans/connects/
  decodes and exposes `Stream<int> bpm`. Paired device id in meta
  (`integration_whoop_device_id`).
- The **input schema** declares what HR drives: `hr_pct` on timer ladder
  entries (auto-stamp elapsed when HR first crosses % of max) and
  `hr_max_target` on timer widgets (session max BPM written on Stop) —
  `src/schema/input.rs`.
- User's max HR: meta key `user_max_hr`, edited on the integration card.
- Values land in visible form fields via the same callback a manual tap
  uses; the normal form save persists them. **No ingest, no provenance** —
  the user is present and can override.

## Adding a new integration (checklist)

1. Decide the pattern (background pull vs live).
2. New class implementing `Integration`; register it in `registry.dart`.
3. Secrets → `flutter_secure_storage`; state/status/cursors → ledger meta
   `integration_<id>_*`.
4. Background pull: define `owned_fields` vs `fill_if_blank_fields`
   thoughtfully (owned = source's own measurements; fill-if-blank = data
   the user might hand-enter), pick a reconcile window, and call
   `ingest()`. Live: drive form fields through existing widget callbacks.
5. Status line: `'Connected · last pulled 08:12 · 143 days synced'` style.
6. If new schema config is needed, extend `src/schema/input.rs` +
   `src/parse/input.rs` (Rust) **and** the app's Dart mirrors
   (`view_schema.dart`, `input_parser.dart`, `engine_schema_adapter.dart`),
   with round-trip parser tests.
