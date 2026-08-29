# airledger-engine

The shared ingest engine for [airledger](https://github.com/oxy-hq/airledger).

A pure-Rust port of the business logic currently living in
`airledger/lib/models/` + `airledger/lib/services/` (Dart):

- Schema parsing for paired `.view.yml` + `.input.yml` files
- Type-aware validation, show-when predicates, derives
- Template interpolation (Jinja-equivalent)
- Sheets ingest (gsheets write path)

Same playbook as
[airlayer](https://github.com/oxy-hq/airlayer): one Rust crate, three
build outputs.

| Target              | Build                  | Binding                  |
|---------------------|------------------------|--------------------------|
| Mobile (Flutter)    | static / dynamic lib   | `dart:ffi`               |
| Web (Oxy bundle)    | WASM (`wasm-bindgen`)  | `@oxy-hq/sdk` consumer   |
| Backend             | native bin / lib       | direct                   |

## Status

- ✅ Phase 1 — schema model + paired `.view.yml` / `.input.yml` parser.
- ✅ Phase 2 — evaluator: show_when predicates, derives, codec,
     minijinja templates.
- ✅ Phase 3 — sheets ingest: RS256 JWT auth + ensure / list /
     create / update / delete.
- ✅ Phase 4a — Dart FFI binding, host build (`sdk-dart/`).
- ✅ Phase 4b — Android / iOS build scripts (`sdk-dart/scripts/`).
- ⏳ Phase 5 — WASM binding for Oxy customer-app bundles.
- ✅ Phase 8 — local-first store + sync engine (`store/`, `sync/`,
     ledger FFI + `EngineLedgerRepository`): SQLite is the app's
     source of truth, Sheets syncs bidirectionally (app wins).
- ✅ Phase 9 — integrations ingest (`store/ingest.rs`): generic
     merge-by-date primitive with provenance + deletion unwind;
     first consumer is the app's Withings → weight pipeline.

See [`docs/port-plan.md`](docs/port-plan.md) for the full plan,
architectural decisions, and how to continue mid-stream.

## Run tests

```sh
cargo test                       # 18 Rust tests (sheets round-trip is env-gated)
cd sdk-dart && dart test         # 6 Dart FFI tests (rebuilds dylib if needed)
```

Run the live-workbook sheets round-trip:

```sh
AIRLEDGER_SHEETS_TEST_CREDS_PATH=/path/to/sa.json \
  AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID=1abc...xyz \
  cargo test --test sheets_integration -- --nocapture
```
