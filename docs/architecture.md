# Airledger architecture (the big picture)

Read this first when returning to the project cold. It explains how the
pieces fit; each section links to a deeper doc or the code.

## The repos

```
~/repos/airledger/           Rust engine (this repo) — schema, eval, store, sync, FFI
~/repos/airledger-archive/   Flutter app (Android) — UI, integrations, connectors
~/repos/airledger-fitness/   LIVE schemas: views/*.view.yml + *.input.yml, templates, ledger.yaml (branding)
~/.config/airledger/         service-account.json + config.yaml (secrets, not in git)
```

Naming gotchas (both matter):

- The Flutter app lives in `airledger-archive` (historical — it predates
  the engine and was "archived" in name only; it is the live, actively
  developed app). The engine repo is `airledger`. The app's own operating
  guide is `~/repos/airledger-archive/CLAUDE.md` — build/deploy loop,
  device serial, Sheets pitfalls.
- The live schema repo is `~/repos/airledger-fitness` (github
  `rsyi/airledger-fitness`). `~/repos/ledger-schemas` is a stale
  predecessor — don't edit it. The app's `tool/sync_assets.sh` has a
  dead default (`~/repos/airledger-schemas`); in practice
  `dart run tool/brand.dart --config ~/repos/airledger-fitness/ledger.yaml`
  sets the source dirs from the config's location and does
  sync + build + install in one shot.

## What the engine is

A pure-Rust port of the app's business logic, built three ways
(`Cargo.toml`): `rlib` (Rust), `cdylib` (Dart FFI), `staticlib`
(iOS/Android native). Modules under `src/`:

| Module    | Purpose |
|-----------|---------|
| `parse/`  | YAML → schema structs for paired `.view.yml` / `.input.yml` |
| `schema/` | The schema model: views, dims, input layer (widgets, timers, ladders) — `schema/input.rs` |
| `eval/`   | show_when predicates, derives, cell codec, minijinja templates |
| `sheets/` | Google Sheets API: RS256 JWT auth, ensure/list/create/update/delete |
| `store/`  | Local-first SQLite ledger + generic ingest primitive — see [local-first-store.md](local-first-store.md) |
| `sync/`   | Bidirectional ledger ↔ Sheets sync engine (app wins conflicts) |
| `ffi.rs`  | C ABI surface consumed by `sdk-dart/` |

`sdk-dart/` wraps the FFI in Dart (`EngineLedgerRepository` etc.), offloads
calls to isolates, and has build scripts for host/Android/iOS dylibs.

## How the app uses it

The app bundles the engine dylib and talks to it via `sdk-dart`:

- `lib/services/engine.dart` — loads the dylib (singleton `getEngine()`).
- `lib/services/engine_ledger_connector.dart` — implements the app's
  `WarehouseConnector` interface over `EngineLedgerRepository`; DB lives at
  `${appDocsDir}/engine_ledger.db`.
- `lib/services/sync_scheduler.dart` — background driver: runs integration
  pulls (`IntegrationRegistry.pullDue()`), then ledger ↔ Sheets sync.

Since phase 8, **the local SQLite ledger is the source of truth** on
device; Sheets is the durable/analyzable mirror, synced bidirectionally
with app-wins conflict resolution. (The app repo's older docs said "Sheets
is the system of record" — that was pre-phase-8.)

Schemas still ship at build time: `airledger-fitness` YAML is copied into
the app's `assets/` by `tool/sync_assets.sh` (invoked via `brand.dart`);
there is no hot-update path — schema edits need a rebuild + reinstall.

## The schema pair

Every tracker is a `.view.yml` (semantic: dims, entities, measures —
portable, shared with oxy/airlayer) plus a `.input.yml` (UI: widgets,
defaults, show_when, timers, plannable — airledger-only). Live copies:
`~/repos/airledger-fitness/views/`. Reference:
`~/repos/airledger-archive/docs/view-input-pairing.md`. Rust structs:
`src/schema/view.rs` / `src/schema/input.rs`. The app has mirror Dart
models (`lib/models/view_schema.dart`, `lib/services/input_parser.dart`,
`lib/services/engine_schema_adapter.dart`) — **schema additions must be
made in both places** (Rust structs + parser, Dart mirrors).

Test fixtures modeling the user's real trackers live in
`tests/fixtures/fitness/` (e.g. `cardio.view.yml` + `cardio.input.yml`,
the 4x4 HIIT tracker with its timer/ladder config).

## External integrations

Pull-based sources (Withings → weight) and live sources (Whoop BLE heart
rate) follow documented patterns — see [integrations.md](integrations.md).

## History / plans

- `README.md` — phase status table.
- `docs/port-plan.md` — the full port plan and architectural decisions.
- `docs/superpowers/specs/` — design specs (local-first sync, Withings,
  Whoop live HR).
- `docs/superpowers/plans/` — implementation plans.

## Tests

```sh
cargo test                    # engine (sheets round-trip is env-gated)
cd sdk-dart && dart test      # FFI binding tests (rebuilds dylib if needed)
```
