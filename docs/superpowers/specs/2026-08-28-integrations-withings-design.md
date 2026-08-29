# External integrations (framework + Withings) — design

**Date:** 2026-08-28
**Status:** approved design, pre-implementation
**Scope:** sub-project 2 (follows the shipped local-first sync,
`2026-08-27-local-first-sync-design.md`). This cut: the integrations
framework (engine ingest primitive + app Integrations page) and the
full Withings pipeline into the weight view. Macrofactor via Android
Health Connect into the meals view is the next integration on this
framework, not in this cut.

## Problem

External data (Withings scale weigh-ins today; Macrofactor nutrition
next) should flow into ledger views automatically. The meals tab is
empty by design — Macrofactor is its intended source. The weight
sheet has a `body_fat_withing` column no schema knows about yet.

## Requirements (validated with user)

- Withings weigh-ins **merge into the day's weight row** (the weight
  view is one row per measurement day): fill `body_fat_withing`
  always; fill `weight_lbs`/`time` only when blank; create the row
  when the day has none.
- Scope: framework + Withings now; the page shows Macrofactor as
  coming soon.
- User registers the Withings developer app (client_id/secret);
  callback URL `airledger://oauth/withings`.
- Deletions in Withings propagate: unwind what Withings contributed
  without touching manual data.
- Integrations write to the **local store**; the existing sync pushes
  to Sheets. No backend.

## Approach (chosen: A)

Engine owns the *ingest* (merge rules, idempotency, provenance,
cursors — pure Rust, table-testable). App owns the *sources*
(Integrations page, OAuth, HTTP pulls, Health Connect later), all
funneling through one engine primitive. Rejected: (B) Withings HTTP
in the engine — token refresh across FFI is awkward and Health
Connect is app-side anyway; (C) backend cron writing to Sheets — no
backend exists and the user wants an in-app integrations page.

## Section 1 — Engine ingest primitive

New FFI: `airledger_engine_ledger_ingest(handle, view_json,
batch_json)` with

```json
{
  "source": "withings",
  "owned_fields": ["body_fat_withing"],
  "fill_if_blank_fields": ["weight_lbs", "time"],
  "records": [ { "date": ..., "time": ..., "weight_lbs": ..., "body_fat_withing": ... } ],
  "deleted_dates": ["2026-08-20"]
}
```

`deleted_dates` is how a source reports unwinds (Section 2): for each
listed day, ingest consults provenance and either deletes the row
(source-created, untouched since) or clears only the source-written
fields. Days without provenance entries are ignored.

Merge, per record, matched on the view's `date_field`:

- No local row that day → create from the record (auto-id, dirty).
- Row exists → owned fields written unconditionally; fill-if-blank
  fields written only where empty. Manual values are never clobbered.
- No-op detection: a record that changes nothing does not dirty the
  row — replaying history is push-free and byte-for-byte harmless.
- Ingest is transactional per batch (`Store::tx`); returns
  `{created, updated, unchanged}`.

Provenance side table (local only, never round-trips the Sheet):
`ingest_provenance(view_name, id, source, fields_json, written_json)`
— which fields a source wrote on which row, and the values it wrote
(to detect manual edits since). Deletion unwind (Section 2) and
disconnect/reconnect idempotency depend on it.

Cursors and per-source status live in the store's `meta` table via
new `ledger_meta_get` / `ledger_meta_set` FFI calls.

## Section 2 — Withings source (Dart)

**OAuth:** browser consent → `airledger://oauth/withings` custom
scheme (intent filter + `app_links`) → code-for-token exchange.
Access token (3 h) + refresh token in `flutter_secure_storage`.
Transparent refresh per pull; dead refresh token → `Reconnect
needed` card status. `client_id`/`client_secret` come from a new
`integrations:` block in `ledger.yaml`, baked in at brand time.

**Pull:** `getmeas` measure types 1 (weight) + 6 (fat ratio) with
`lastupdate = cursor`. Transform: kg → lbs (1 dp), fat ratio →
`body_fat_withing` (1 dp), epoch → local date + HH:MM. One record
per day: **earliest weigh-in wins**; later same-day readings are
ignored. Batch → `ledger_ingest`; cursor advances only after the
ingest commits.

**First connect = full backfill** of Withings history. Idempotent,
interruptible, re-runnable.

**Deletions:** each pull also reconciles a rolling **90-day window**:
days provenance credits to Withings but Withings no longer has →
unwound by ingest: row created by Withings and untouched since
(current data still equals provenance's `written_json`) → delete the
row (sync tombstones it off the Sheet); otherwise clear only the
Withings-written fields. API-reported deletions (`lastupdate`
returns deleted groups) are honored when present; the window is the
catch-all. Deletions older than the window need the page's **Full
reconcile** action (all-history sweep) or a manual delete.

## Section 3 — Integrations page + scheduling

Home screen gains an **Integrations** entry (next to Apps). The page
lists integrations as cards:

- Name + target ("Withings → weight"; "Macrofactor → meals" greyed
  out as coming soon).
- Status: `Not connected` / `Connected · last pulled … · N days
  synced` / `Reconnect needed` / `Error: …` — read from store `meta`.
- Actions: `Connect` (OAuth) when disconnected; `Sync now`, and an
  overflow with `Full reconcile` and `Disconnect` (wipes tokens +
  cursor; provenance stays) when connected.

**Scheduling:** no new scheduler. Pulls ride the existing
`SyncScheduler` triggers (app start, resume, wifi-appears) with a
**6-hour per-source minimum interval**; `Sync now` bypasses it. All
gated by the existing wifi-only setting. Cycle order: **pull sources
→ ingest → ledger sync**, so a weigh-in reaches the Sheet in the
same cycle. A source failure sets its card status and retries next
trigger; it never delays or fails the ledger sync.

## Section 4 — Schema, errors, testing

**Schema:** `weight.view.yml` gains
`{ name: body_fat_withing, type: number, expr: body_fat_withing }`;
`weight.input.yml` shows it as an optional number field. Mirrored in
ledger-schemas; pushed to GitHub for the app's schema sync.
`ensure_sheet` reconciles the column (already present in the sheet).

**Errors:** OAuth failures → card status only. Refresh failure →
`Reconnect needed`; cursor + provenance survive reconnects.
Pull/transform failures → per-source status, retry next trigger.
Ingest transactional per batch; cursor advances only post-commit, so
crashes re-deliver and idempotency absorbs. Unwind never touches
rows provenance can't vouch for.

**Testing:**
- Rust: table-driven ingest tests (create/fill/owned-overwrite/no-op/
  replay/unwind both provenance cases/cursor semantics) + ingest →
  sync round-trip via `FakeRemote`.
- Dart: Withings fixture → transform tests (unit conversion,
  earliest-of-day, deletion-set derivation). OAuth manually on
  device.
- On-device: connect → backfill → merged weight history → sheet
  column populated → delete a weigh-in in Withings → next pull
  unwinds it.

## Out of scope (recorded for later)

- Macrofactor via Health Connect → meals (next integration; uses the
  same ingest primitive with `owned_fields: [calories, protein_g]`).
- Optional import of the "MFP Archive (2012-07-31-to-2024-06-30)"
  tab (old cardio spreadsheet) into meals.
- Multi-device, backend-hosted pulls.
