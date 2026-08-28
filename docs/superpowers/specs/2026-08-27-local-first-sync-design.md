# Local-first storage + wifi-gated cloud sync — design

**Date:** 2026-08-27
**Status:** approved design, pre-implementation
**Scope:** sub-project 1 of 2. Sub-project 2 (external integrations:
Macrofactor via Health Connect, Withings API) is deliberately out of
scope and will be brainstormed separately once this ships — it will
write into the local store defined here and get Sheets sync for free.

## Problem

The app reads and writes Google Sheets directly on every screen load
and every edit. That makes it slow (every interaction is a network
round-trip) and data-hungry (full-tab reads over cellular). There is
no local persistence at all.

## Requirements (validated with user)

- Single device (one Pixel phone) runs the app.
- Local DB becomes the source of truth; the app is fully usable
  offline. All reads/writes are local.
- Sync is bidirectional: the user occasionally edits the Sheet
  directly (desktop browser), and those edits must flow back.
- Conflict rule when the same row changed on both sides since last
  sync: **app wins** (the phone's version overwrites the Sheet).
- Sync runs automatically on app use — app open, app resume,
  debounced after writes, and when wifi appears with pending changes.
  No OS background service.
- Wifi gating is a setting: `Sync on Wi-Fi only`, default **on**;
  turning it off allows cellular.

## Approach (chosen: A)

Local store + sync engine live in the Rust crate (this repo), per the
port-plan architecture: business logic in Rust, shared by mobile now
and WASM/backend later. Rejected alternatives: (B) Dart-side store in
the archive app — moves logic back into Dart, the thing the rewrite
is undoing; (C) read-cache + write-through queue — half-measure with
a murky source-of-truth story that would be rebuilt as (A) later.

## Section 1 — Local data model

New `store` module: bundled SQLite via `rusqlite`. One DB file; path
supplied by the app at engine load (Flutter app-documents dir).

Ledger schemas are dynamic YAML, so no per-sheet DDL. One generic
table:

```sql
rows(
  view_name  TEXT NOT NULL,      -- e.g. "weight", "meals"
  id         TEXT NOT NULL,      -- record UUID
  data       TEXT NOT NULL,      -- Record as the existing CellValue JSON envelope
  base       TEXT,               -- remote copy as of last sync (NULL = never synced/new)
  dirty      INTEGER NOT NULL,   -- 1 = local change not yet pushed
  deleted    INTEGER NOT NULL,   -- tombstone: deleted locally, not yet pushed
  updated_at TEXT NOT NULL,      -- informational only; never used for merge
  PRIMARY KEY (view_name, id)
)
meta(key TEXT PRIMARY KEY, value TEXT)  -- last-sync per view, schema_version
```

`base` is the three-way-merge anchor: remote-vs-`base` detects remote
changes, `dirty` detects local changes.

Local CRUD semantics (what the app calls):

- `list` — read + decode from SQLite only. Date filtering and
  time-of-day sorting carried over from `SheetsRepository::list`
  unchanged.
- `create` — insert with `dirty=1`, `base=NULL`; UUID auto-assigned
  when the view declares an `id` dimension (same rule as today).
- `update` — overwrite `data`, set `dirty=1`.
- `delete` — tombstone (`deleted=1`) if ever synced (`base` non-NULL),
  else remove the row outright.
- `__row` disappears from the app's world. The store addresses purely
  by `id`; sheet row positions are a sync-time concern only.

Remote rows lacking an id (hand-typed in the Sheet) get a UUID
assigned during sync and written back, so everything is addressable.

## Section 2 — Sync algorithm

One engine call, run per view in sequence: **pull → merge → push →
commit**. Interrupt-safe at every point: dirty flags clear only after
a confirmed push, so a failed sync retries next trigger.

1. **Pull.** One `get_values` for the whole tab — the only remaining
   full-sheet read in the app. Decode rows, index by `id`. Id-less
   remote rows get UUIDs queued for write-back.
2. **Merge** — pure function, per id, three-way against `base`.
   No I/O in this step:

   | Local state | Remote vs `base` | Action |
   |---|---|---|
   | clean | changed | take remote → update local `data` |
   | dirty | unchanged | push local |
   | dirty | changed | **app wins** → push local |
   | clean | row gone | delete locally |
   | tombstone | — | delete remote row |
   | new (`base=NULL`) | — | insert remote at top |
   | clean | unchanged | nothing |

3. **Push.** Apply decisions with the existing repo ops
   (`insertDimension`, ranged `update_values`, `deleteDimension`),
   positions computed from the just-pulled snapshot; deletes applied
   bottom-up so indexes stay valid. `ensure_sheet` runs first so
   schema-added columns materialize.
4. **Commit.** Per reconciled row: `base` := what the Sheet now
   holds, `dirty` := 0, purge tombstones. Record sync time in `meta`.

Edge cases: mid-sync failure leaves dirty rows dirty (retry-safe);
duplicate ids in the Sheet — first wins, rest ignored; auth/network
errors abort that view's sync without touching local data.

Accepted race (single user): editing the Sheet *while* a sync runs
can stale the push positions — same exposure as today's `__row`
addressing. Not engineered around.

## Section 3 — FFI surface and app-side wiring

**Engine side** — one new handle, following the `sheets_connect`
pattern:

- `airledger_engine_ledger_open(db_path, service_account_json,
  spreadsheet_id)` → handle wrapping the SQLite store and a
  lazily-built `SheetsRepository` (network untouched until sync).
- Local CRUD: `..._ledger_list` / `..._ledger_create` /
  `..._ledger_update` / `..._ledger_delete` — same JSON record
  envelopes as the sheets FFI, so Dart bindings are near copy-paste.
- `airledger_engine_ledger_sync(handle, views_json)` → summary JSON
  (`{pushed, pulled, conflicts, errors}` per view).
- `airledger_engine_ledger_pending(handle)` → dirty-row count.
- `airledger_engine_ledger_free_handle`.

**App side (archive repo)** — `EngineLedgerRepository` implements the
same `WarehouseConnector` interface as `EngineSheetsRepository`; the
UI is unchanged and just receives a local-backed connector. Runs on
the existing worker isolate.

New Dart pieces, all thin:

- `SyncScheduler` — listens to `connectivity_plus`, owns the
  triggers: app start, app resume, ~5 s debounce after local writes,
  wifi-appears-with-pending-changes. One sync at a time; a trigger
  during a running sync is skipped.
- Setting: `Sync on Wi-Fi only` toggle (default on; off allows
  cellular), stored in shared prefs with existing settings.
- Status: last-synced timestamp + pending count shown unobtrusively
  (e.g. ledger drawer). No blocking spinners.

**First run:** local DB is empty; the initial sync hydrates it via
the normal merge path (every remote row is "remote new"). Requires
connectivity once; before that the app runs with an empty ledger
rather than erroring.

## Section 4 — Error handling and edge cases

- Failed sync (offline, auth, 5xx): per-view error in the sync
  summary, dirty flags intact, retry at next trigger. User-facing
  signal is the pending count / stale last-synced stamp — no error
  dialogs for background work.
- Local CRUD never touches the network. SQLite errors surface via
  the existing `error_json` FFI envelope.
- Partial push (process dies mid-push): next sync re-pulls, sees
  remote already matches local, decides "nothing to push," commits.
  Self-healing, no duplicates — pushes are keyed by id.
- Schema evolution: a new dimension (e.g. `body_fat_withing` on the
  weight view — note: schema not yet updated in airledger-fitness;
  reconcile column name vs the Sheet when implementing) is just a
  new key in `data`; `ensure_sheet` adds the column remotely at next
  sync. Unknown sheet columns preserved verbatim on push, as today.
- DB migrations: `schema_version` in `meta`; v1 ships the two
  tables. `PRAGMA journal_mode=WAL` against mid-write kills.
- No timestamp comparisons anywhere in merge — correctness rests on
  `base` diffs and `dirty` flags, immune to clock skew.

## Section 5 — Testing

- **Merge logic**: pure `(local rows, remote snapshot) → actions`
  function; table-driven Rust unit tests covering every matrix row
  plus id-less rows, duplicate ids, tombstone-vs-remote-edit.
- **Store CRUD**: rusqlite against tempfile/in-memory DB —
  round-trips, dirty/tombstone bookkeeping, date-filter + time-sort
  parity with current `list()`.
- **End-to-end sync**: env-gated live test alongside
  `sheets_integration` — seed sheet, sync to fresh DB, mutate both
  sides, sync again, assert matrix outcomes.
- **Dart FFI smoke tests**: extend `sdk-dart/test/smoke_test.dart` —
  open ledger handle, CRUD with no network, pending count moves.
- **On-device**: strength view rendering from the local store on the
  Pixel, then a live sync round-trip on wifi (same bar as phase 6d).
