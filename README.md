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

- Phase 1 — schema model + YAML parser. Tested against the real
  airledger-fitness + pokehouse-ledger schemas as fixtures.
- Phase 2 — validation, show-when, derives, template apply.
- Phase 3 — sheets ingest (writes).
- Phase 4 — FFI + WASM bindings.

See `docs/architecture.md` (TODO) for the full plan.

## Run tests

```sh
cargo test
```
