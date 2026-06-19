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
- ⏳ Phase 3 — sheets ingest (writes).
- ✅ Phase 4a — Dart FFI binding, host build (`sdk-dart/`).
- ⏳ Phase 4b — Dart FFI Android / iOS builds.
- ⏳ Phase 5 — WASM binding for Oxy customer-app bundles.

See [`docs/port-plan.md`](docs/port-plan.md) for the full plan,
architectural decisions, and how to continue mid-stream.

## Run tests

```sh
cargo test                       # 15 Rust tests
cd sdk-dart && dart test         # 6 Dart FFI tests (rebuilds dylib if needed)
```
