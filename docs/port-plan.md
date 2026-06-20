# Port plan — Dart Flutter → Rust engine + bindings

This doc captures the *why* and the *where we are* of the airledger
rewrite, so anyone (including future-Claude) walking in cold can pick
up the work.

## Background

Through mid-2026 airledger was a Flutter mobile app written in Dart.
Schema-driven CRUD over Google Sheets — `.view.yml` + `.input.yml`
defines a tracker; the app generates the form, timeline, history
panel, recipe production view, and so on.

That codebase is preserved verbatim in
[`oxy-hq/airledger-archive`](https://github.com/oxy-hq/airledger-archive)
(GitHub) and `~/repos/airledger-archive` (locally). It's still the
shipping mobile app — installing on the Pixel still goes through
that repo via `dart run tool/brand.dart --config <ledger.yaml>`.

This repo (`oxy-hq/airledger`, `~/repos/airledger`) holds the Rust
port — the *shared engine*. The architectural goal:

```
                     +--------------------------+
                     |  Rust crate: ingest      |
                     |  (schema, validate,      |
                     |   derives, sheets I/O)   |
                     +--------------------------+
                       |              |        |
              FFI dylib|         WASM |        | Native
                       v              v        v
                +------------+  +-----------+  +-----------+
                | Mobile app |  | Oxy bundle|  | Backend   |
                | (Flutter / |  | (React via|  | services  |
                |  RN later) |  |  WASM)    |  +-----------+
                +------------+  +-----------+
```

Same playbook as [airlayer](https://github.com/oxy-hq/airlayer) +
[its `sdk-dart`](https://github.com/oxy-hq/airlayer/tree/main/sdk-dart):
one Rust crate, three build outputs.

The driver was *shared business logic across mobile and the Oxy
customer-app bundle*. Flutter Web could put the mobile UI on the web
but doesn't share *logic*, so it's the wrong tool. React Native would
unify mobile + web at the cost of dropping the Flutter polish — also
not necessary if we extract the engine cleanly.

## Phase status

| Phase | Scope | Status |
|---|---|---|
| 1 | Schema model + `.view.yml` / `.input.yml` parser | ✅ shipped — commit `bfb2409` |
| 2 | Evaluator layer: `show_when`, derives, codec, minijinja | ✅ shipped — commit `e9f3e64` |
| 3 | Sheets ingest (ensure + list + create + update + delete + JWT auth) | ✅ shipped — this checkpoint |
| 4a | Dart FFI binding (host build only) | ✅ shipped — commit `64a5883` |
| 4b | Dart FFI Android / iOS build scripts | ✅ shipped — Android validated, iOS scripts ready (needs full Xcode) |
| 5 | WASM binding for Oxy customer-app bundle | ⏳ not started |
| 6a | Smoke proof: engine loads + answers `version` on the device | ✅ shipped |
| 6b | Schema cutover: `schema_parser` + `input_parser` routed through engine | ✅ shipped — `f790111` / `ef6785c` in archive |
| 6c | Sheets FFI + Dart wrapper (`EngineSheetsRepository`) | ✅ shipped |
| 6d | Archive sheets consumer cutover — UI typed against `WarehouseConnector`, engine-routed | ✅ shipped — validated on Pixel: strength view's 10 rows render via engine `list()` |
| 7 | Drop in the engine on a React Oxy bundle (WASM consumer) | ⏳ not started |

## What lives where

```
~/repos/airledger/              <- Rust engine (this repo)
├── Cargo.toml                  serde + serde_yaml + chrono + minijinja + thiserror + serde_json
├── src/
│   ├── lib.rs                  re-exports
│   ├── value.rs                CellValue enum + Record = BTreeMap<String, CellValue>
│   ├── schema/
│   │   ├── view.rs             ViewSchema + Dimension/Measure/Entity types
│   │   ├── input.rs            InputSpec + TimerLadder/TimerStopTarget/RepeatGroup
│   │   └── overlay.rs          InputOverlay + apply_overlay + OverlayError
│   ├── parse/
│   │   ├── view.rs             .view.yml → ViewSchema
│   │   └── input.rs            .input.yml → InputOverlay (flat + legacy layouts)
│   ├── eval/
│   │   ├── codec.rs            encode + decode (Sheets ↔ CellValue)
│   │   ├── show_when.rs        is_visible_given (form hide/show predicate)
│   │   ├── derive.rs           apply_derives + run_derive
│   │   └── template.rs         minijinja with custom `round` filter
│   ├── sheets/                 Sheets ingest (Phase 3)
│   │   ├── auth.rs             service-account → RS256 JWT → access token
│   │   ├── api.rs              REST wrappers (get_spreadsheet, get_values,
│   │   │                          update_values, batch_update)
│   │   └── repo.rs             SheetsRepository: ensure_sheet, list,
│   │                              create, update, delete
│   └── ffi.rs                  C-ABI entry points (parse only; sheets
│                                 FFI deferred to Phase 6 wiring)
├── sdk-dart/                   Dart wrapper, mirrors airlayer/sdk-dart
│   ├── lib/airledger_engine.dart   public API (AirledgerEngine.load(), parseView, ...)
│   ├── lib/src/bindings.dart       raw FFI symbol lookups
│   ├── lib/src/airledger_engine_base.dart   impl
│   ├── test/smoke_test.dart        6 end-to-end FFI tests
│   └── scripts/
│       ├── build-host.sh           cargo build (host platform)
│       ├── build-android.sh        cargo-ndk for arm64/armv7/x86_64/x86
│       └── build-ios.sh            cargo rustc + lipo + xcframework
├── tests/
│   ├── fixtures/                copies of real fitness + pokehouse YAMLs
│   │   ├── fitness/             8 .input.yml + 12 .view.yml + templates
│   │   └── pokehouse/           3 .input.yml + 2 .view.yml + templates
│   ├── parse_real_schemas.rs    5 Phase-1 tests
│   ├── eval.rs                  10 Phase-2 tests
│   ├── sheets_unit.rs           3 Phase-3 unit tests (no network)
│   └── sheets_integration.rs    1 Phase-3 round-trip (env-gated)
└── docs/port-plan.md            this file

~/repos/airledger-archive/      <- Dart Flutter app (legacy mobile, still shipping)
~/repos/airledger-fitness/      <- schemas for my personal fitness deploy
~/repos/pokehouse-ledger/       <- schemas for the Poke House inventory deploy
~/repos/airlayer/               <- the *other* engine — semantic layer, our reference for FFI+WASM patterns
```

## How to run

```sh
cd ~/repos/airledger
cargo test                 # 18 Rust tests (3 + 5 + 10 + 1-gated)
cargo build                # builds rlib + cdylib + staticlib

cd sdk-dart
./scripts/build-host.sh    # rebuilds the engine for the dart loader
dart pub get               # install dart deps
dart test                  # 6 FFI smoke tests
./scripts/build-android.sh # only when targeting Android device
./scripts/build-ios.sh     # only when targeting iOS device / sim
```

For the Phase 3 sheets round-trip test against a real workbook:

```sh
export AIRLEDGER_SHEETS_TEST_CREDS_PATH=/path/to/service-account.json
export AIRLEDGER_SHEETS_TEST_SPREADSHEET_ID=1abc...xyz
cargo test --test sheets_integration -- --nocapture
```

## Key architectural decisions (so future-you doesn't have to re-derive)

- **JSON over the FFI**, not byte-compatible structs. ABI stays flat;
  serialize + parse is < 1ms at the form's interactivity budget.
- **`CellValue` is a tagged union** of `Null / Bool / Int / Float /
  String / Date / DateTime` — typed equivalent of Dart's `Object?`.
- **`Record = BTreeMap<String, CellValue>`** is the row shape
  throughout. Same surface as Dart's `Map<String, Object?>`, just
  typed.
- **Errors come back as `{"error": "..."}` JSON** from the FFI, and
  the Dart side unpacks to `EngineError`. Successful calls return
  the raw serialized result (no wrapper). Lets the consumer use the
  same `_decode` path for both kinds of return.
- **`Environment::empty()` for minijinja** (not `Environment::new()`)
  — we register only the filters we use (custom `round`) and rely on
  the parity tests against Dart's behavior, not on inherited
  filter behavior.
- **The schema YAMLs are the source of truth.** Both the archived
  Flutter app and this Rust engine read the same `.view.yml` +
  `.input.yml` files from `airledger-fitness` and `pokehouse-ledger`.
  Schema changes are made in those repos; both consumers update.
- **Sheets ingest is sync, not async.** `reqwest::blocking` + a
  shared client across the repository. FFI is sync so async would
  just add complexity. The WASM binding (Phase 5) will need an
  async fetch-based variant — that's a separate module.
- **Sheets retry policy: only transport failures.** Mirrors the
  Dart `_RetryingClient`: 4 attempts max, exponential backoff
  starting at 300ms, retry only on `is_connect()` / `is_timeout()`.
  Never retry 4xx/5xx — non-idempotent writes stay safe.
- **Sheets FFI surface deferred.** Phase 3 ships the Rust API and
  an env-gated round-trip test. Wiring sheets into Dart is Phase 6
  (the cutover); doing it now would mean designing the
  state-handle ABI without a consumer asking for it.

## What's next, in order

1. **Phase 5 — WASM binding.** Add `wasm-bindgen` exports next to
   `src/ffi.rs` (or `src/wasm.rs`). Build via `wasm-pack build
   --target web`. Validate from a minimal JS harness that
   `parse_view_pair` returns the same JSON the Dart side gets.
   Sheets module needs a fetch-based async variant for WASM (or
   gate it off the wasm32 target and let the JS consumer call
   Google APIs directly via fetch).
2. **Phase 6b — real Flutter cutover.** Phase 6a (this checkpoint)
   only proves the engine loads. The actual swap-in is harder:
   - JSON → Dart `ViewSchema` converter (the engine returns the
     same shape, but as `Map<String, dynamic>`; the rest of the
     Flutter app uses Dart classes).
   - Route `schema_parser.dart` + `input_parser.dart` through the
     engine, behind a feature flag.
   - Design the sheets FFI handle: `airledger_engine_sheets_connect`
     returns an opaque `*mut SheetsRepository`; Dart wraps with
     a `Finalizer` so Rust drops it on GC. Methods
     `_ensure / _list / _create / _update / _delete` take the
     handle + JSON args.
   - Cut over sheets last (state-handle ABI carries more risk
     than the stateless parsers).
3. **Phase 7 — Oxy consumer.** Drop the WASM module into a React
   customer-app bundle and call from the Oxy SDK. Whatever proves
   the round trip works end-to-end.

iOS build (`build-ios.sh`) needs full Xcode (only Command Line
Tools were on this machine when 4b was validated). Android works.
This is fine for now since the target device is a Pixel 10 Pro.

## How to continue from here

If you're picking this up after a context wipe:

1. `cd ~/repos/airledger && cargo test` — make sure the 18 Rust
   tests pass (3 sheets unit + 5 parse + 10 eval + 1 integration
   that skips without env).
2. `cd sdk-dart && dart test` — make sure the 6 Dart FFI tests pass.
   If the engine was rebuilt, run `./scripts/build-host.sh` first.
3. Open this doc + `tests/parse_real_schemas.rs` + `tests/eval.rs`
   + `tests/sheets_unit.rs` + `sdk-dart/test/smoke_test.dart` to
   get a feel for the API surface and the parity guarantees with
   the Dart side.
4. Pick the next phase from the list above. They're roughly ordered
   by dependency / risk.
5. The Dart-side reference for the sheets module lives in
   `~/repos/airledger-archive/lib/services/sheets_repository.dart`.
   Compare to `src/sheets/repo.rs` to confirm parity on edge cases
   (especially the `__row` resolution + additive header merge).
