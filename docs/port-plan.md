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
| 3 | Sheets ingest (ensureSheet + read + create + update + delete) | ⏳ not started |
| 4a | Dart FFI binding (host build only) | ✅ shipped — this checkpoint |
| 4b | Dart FFI Android / iOS builds | ⏳ not started |
| 5 | WASM binding for Oxy customer-app bundle | ⏳ not started |
| 6 | Drop in the engine on the Flutter mobile app (sdk-dart consumer) | ⏳ not started |
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
│   └── ffi.rs                  C-ABI entry points
├── sdk-dart/                   Dart wrapper, mirrors airlayer/sdk-dart
│   ├── lib/airledger_engine.dart   public API (AirledgerEngine.load(), parseView, ...)
│   ├── lib/src/bindings.dart       raw FFI symbol lookups
│   ├── lib/src/airledger_engine_base.dart   impl
│   ├── test/smoke_test.dart        6 end-to-end FFI tests
│   └── scripts/build-host.sh       cargo build (host platform)
├── tests/
│   ├── fixtures/                copies of real fitness + pokehouse YAMLs
│   │   ├── fitness/             8 .input.yml + 12 .view.yml + templates
│   │   └── pokehouse/           3 .input.yml + 2 .view.yml + templates
│   ├── parse_real_schemas.rs    5 Phase-1 tests
│   └── eval.rs                  10 Phase-2 tests
└── docs/port-plan.md            this file

~/repos/airledger-archive/      <- Dart Flutter app (legacy mobile, still shipping)
~/repos/airledger-fitness/      <- schemas for my personal fitness deploy
~/repos/pokehouse-ledger/       <- schemas for the Poke House inventory deploy
~/repos/airlayer/               <- the *other* engine — semantic layer, our reference for FFI+WASM patterns
```

## How to run

```sh
cd ~/repos/airledger
cargo test                 # 15 Rust tests (parse + eval)
cargo build                # builds rlib + cdylib + staticlib

cd sdk-dart
./scripts/build-host.sh    # rebuilds the engine for the dart loader
dart pub get               # install dart deps
dart test                  # 6 FFI smoke tests
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

## What's next, in order

1. **Phase 3 — sheets ingest (write path).** Port
   `sheets_repository.dart` to Rust: ensureSheet (additive header
   merge), create, update, delete, list. Service-account JWT
   signing needs `jsonwebtoken` + `ring` (or use `reqwest` +
   `gcp_auth` if it exists). Add a `tests/sheets.rs` integration
   test against a throwaway test workbook gated behind an env var.
2. **Phase 4b — Android / iOS dylib builds.** Mirror
   `airlayer/sdk-dart/scripts/build-android.sh` and `build-ios.sh`
   using `cargo-ndk` and `cargo lipo`. Copy resulting `.so` /
   `.dylib` into `sdk-dart/build/jniLibs/<abi>/` matching the
   Flutter `jniLibs` layout.
3. **Phase 5 — WASM binding.** Add `wasm-bindgen` exports next to
   `src/ffi.rs` (or `src/wasm.rs`). Build via `wasm-pack build
   --target web`. Validate from a minimal JS harness that
   `parse_view_pair` returns the same JSON the Dart side gets.
4. **Phase 6 — Flutter consumer.** In the archive repo's
   `pubspec.yaml`, add `airledger_engine: { path:
   ../airledger/sdk-dart }`. Replace `lib/services/schema_parser.dart`
   and `lib/services/input_parser.dart` with a thin wrapper that
   calls the engine. Cut over one parser at a time; both
   implementations can coexist behind a feature flag.
5. **Phase 7 — Oxy consumer.** Drop the WASM module into a React
   customer-app bundle and call from the Oxy SDK. Whatever proves
   the round trip works end-to-end.

## How to continue from here

If you're picking this up after a context wipe:

1. `cd ~/repos/airledger && cargo test` — make sure the 15 Rust
   tests pass.
2. `cd sdk-dart && dart test` — make sure the 6 Dart FFI tests pass.
   If the engine was rebuilt, run `./scripts/build-host.sh` first.
3. Open this doc + `tests/parse_real_schemas.rs` + `tests/eval.rs`
   + `sdk-dart/test/smoke_test.dart` to get a feel for the API
   surface and the parity guarantees with the Dart side.
4. Pick the next phase from the list above. They're roughly ordered
   by dependency / risk.
5. The Dart-side reference for every Phase 3 piece lives in
   `~/repos/airledger-archive/lib/services/sheets_repository.dart`.
   Read that, then port. The shape of the Sheets API calls is well-
   trodden — see `tool/migrate_4x4_to_correct_sheet.dart` in the
   archive for an example of the JWT + sheets API pattern.
