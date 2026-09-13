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
| `../ledger/CLAUDE.md` | The Flutter app's operating guide: build/deploy loop, device serial, asset pipeline, Sheets pitfalls |
| `../ledger/docs/view-input-pairing.md` | `.view.yml` ↔ `.input.yml` schema pairing rules |

## Ground rules

- **After any engine change the app consumes, rebuild the Android dylib:**
  `cd sdk-dart && ./scripts/build-android.sh`, then rebuild/install the
  APK. The app bundles `sdk-dart/build/jniLibs/*.so` at build time and
  parses schemas through it (`useEngine = true`) — a stale dylib
  silently drops new schema keys with no error. Sanity check:
  `strings sdk-dart/build/jniLibs/arm64-v8a/libairledger_engine.so | grep <new_key>`.
- **Push `airledger-fitness` after schema edits.** The app's SchemaSync
  pulls `views/` from GitHub and prefers the synced copy over bundled
  assets — an unpushed schema change gets reverted on device at the
  next sync poll.

- **Schema changes go in two places:** Rust (`src/schema/`, `src/parse/`)
  and the app's Dart mirrors (`lib/models/view_schema.dart`,
  `lib/services/input_parser.dart`, `lib/services/engine_schema_adapter.dart`).
  Add round-trip parser tests; update `tests/fixtures/fitness/` if the
  user's real trackers gain the feature.
- **The local ledger is the source of truth** (post phase 8); Sheets is
  the synced mirror, app-wins. Don't reintroduce "Sheets is truth"
  assumptions from older docs.
- The Flutter app repo is `~/repos/ledger` (GitHub `rsyi/ledger`,
  formerly `oxy-hq/airledger-archive`); the product name is "Ledger".
- Commits in this repo: conventional style (`feat(ingest): …`,
  `docs: …`), commit when a phase/step completes.

## Tests

```sh
cargo test                    # engine tests (sheets round-trip env-gated)
cd sdk-dart && dart test      # Dart FFI tests (rebuilds dylib if needed)
```
