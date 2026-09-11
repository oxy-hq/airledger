# CLAUDE.md — airledger engine

Rust engine for the airledger local-first personal tracker. This file is
an index; the docs below hold the substance. Read
[docs/architecture.md](docs/architecture.md) first if you're new or
returning cold.

## Doc index

| Doc | What it covers |
|-----|----------------|
| [docs/architecture.md](docs/architecture.md) | The big picture: repos (engine / Flutter app / schemas), engine modules, FFI + sdk-dart, how the app consumes the engine |
| [docs/local-first-store.md](docs/local-first-store.md) | SQLite ledger data model, meta table, ledger ↔ Sheets sync, the `ingest()` primitive + provenance/deletion unwind |
| [docs/integrations.md](docs/integrations.md) | Background-pull (Withings) vs live (Whoop BLE HR) integration patterns, `Integration` interface, add-a-new-integration checklist |
| [docs/port-plan.md](docs/port-plan.md) | Full port plan + architectural decisions, phase by phase |
| [docs/superpowers/specs/](docs/superpowers/specs/) | Design specs (local-first sync, Withings, Whoop live HR) |
| [docs/superpowers/plans/](docs/superpowers/plans/) | Implementation plans |
| `../airledger-archive/CLAUDE.md` | The Flutter app's operating guide: build/deploy loop, device serial, asset pipeline, Sheets pitfalls |
| `../airledger-archive/docs/view-input-pairing.md` | `.view.yml` ↔ `.input.yml` schema pairing rules |

## Ground rules

- **Schema changes go in two places:** Rust (`src/schema/`, `src/parse/`)
  and the app's Dart mirrors (`lib/models/view_schema.dart`,
  `lib/services/input_parser.dart`, `lib/services/engine_schema_adapter.dart`).
  Add round-trip parser tests; update `tests/fixtures/fitness/` if the
  user's real trackers gain the feature.
- **The local ledger is the source of truth** (post phase 8); Sheets is
  the synced mirror, app-wins. Don't reintroduce "Sheets is truth"
  assumptions from older docs.
- The Flutter app repo is `~/repos/airledger-archive` — live and actively
  developed despite the name.
- Commits in this repo: conventional style (`feat(ingest): …`,
  `docs: …`), commit when a phase/step completes.

## Tests

```sh
cargo test                    # engine tests (sheets round-trip env-gated)
cd sdk-dart && dart test      # Dart FFI tests (rebuilds dylib if needed)
```
