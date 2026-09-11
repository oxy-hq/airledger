# Local-first store & sync (phase 8/9)

How the on-device ledger works. Code: `src/store/`, `src/sync/`; design
spec: `docs/superpowers/specs/2026-08-27-local-first-sync-design.md`.

## Data model (`src/store/db.rs`, `src/value.rs`)

- One generic SQLite `rows` table, polymorphic by view name; each row is a
  `BTreeMap<String, CellValue>` (mirrors the Dart `Map<String, Object?>`).
- `CellValue`: Null / Bool / Int / Float / String / Date (`YYYY-MM-DD`) /
  DateTime. No native high-frequency time-series type — grain is the
  view's primary entity (typically one row per day or per session), and
  time-of-day lives in string fields.
- Per-row sync metadata: `__row` (sheet row number), `dirty` (local change
  pending push), `base` (remote copy at last sync, for three-way merge),
  `deleted` (tombstone).
- A `meta` key/value table, exposed through FFI meta get/set. Used for
  sync cursors, integration status (`integration_<id>_*`), and user
  settings (e.g. `user_max_hr`). The app UI and engine share it as one
  source of truth.

## Sync (`src/sync/`)

Bidirectional ledger ↔ Sheets. Local edits mark rows `dirty` and push on
the next sync; remote edits are pulled and three-way-merged against
`base`; conflicts resolve **app-wins**. Tombstoned rows delete the sheet
row. The app's `sync_scheduler.dart` decides when to run (connectivity,
app lifecycle, manual "Sync now").

## Ingest primitive (`src/store/ingest.rs`)

`ingest()` is the generic merge-by-date entry point for external sources
(FFI: `airledger_engine_ledger_ingest`; Dart:
`EngineLedgerRepository.ingest`). Input:

```rust
pub struct IngestBatch {
    pub source: String,                    // "withings"
    pub owned_fields: Vec<String>,         // source always wins these
    pub fill_if_blank_fields: Vec<String>, // source fills only if empty
    pub records: Vec<Record>,              // one per date (view's date_field)
    pub deleted_dates: Vec<String>,        // reconcile: unwind these dates
}
```

Returns counts: `{created, updated, unchanged, skipped, deleted, cleared}`.

### Provenance

Every ingest write is recorded per (view, row, source): which fields, the
exact values written, and whether the source created the row. This enables
**deletion unwind**: when a source reports a date deleted, the engine
- deletes the row entirely if the source created it and its fields are
  untouched (tombstone syncs to the sheet), or
- clears only the source-written fields, preserving user edits.

User edits always survive: `owned_fields` only overwrite what provenance
says the source itself wrote.

## What ingest is NOT for

Ingest + provenance serve **background pull** integrations. Interactive
flows where the user is present in a form (e.g. Whoop live HR stamping
fields during a workout) go through the normal form-edit path — the value
lands in a form field the user can see and override, and the ordinary row
save persists it. See [integrations.md](integrations.md).
