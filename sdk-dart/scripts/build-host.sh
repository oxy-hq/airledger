#!/usr/bin/env bash
# Build the airledger-engine dylib for the host platform.
# Run before `dart test` in sdk-dart/ — the Dart loader expects the
# library to be at `target/debug/` or `target/release/` relative to
# the engine crate root.
#
# Mirrors airlayer/sdk-dart/scripts/build-host.sh.

set -euo pipefail
cd "$(dirname "$0")/../.."   # engine crate root

if [ "${1:-}" = "--release" ]; then
  cargo build --release
else
  cargo build
fi

echo
echo "host libs ready in target/{debug,release}/"
find target -maxdepth 2 -name 'libairledger_engine.dylib' \
              -o -name 'libairledger_engine.so' \
              -o -name 'airledger_engine.dll' 2>/dev/null \
  | sed 's/^/  /'
