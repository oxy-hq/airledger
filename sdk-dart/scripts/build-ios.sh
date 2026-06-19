#!/usr/bin/env bash
# Build a static archive (.a) for iOS device + simulator and bundle as an
# xcframework so Flutter iOS apps can link it.
#
# Prereqs:
#   - Xcode + command-line tools
#   - Rust targets:
#       rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
#
# We build a static lib (not a dylib) because Apple's app review process
# rejects loose .dylibs. The lib gets statically linked into the host app.
# In Dart, `AirledgerEngine.load()` on iOS uses
# `ffi.DynamicLibrary.process()` (rather than file path lookup) so it
# finds the symbols already linked into the process image.
#
# Mirrors airlayer/sdk-dart/scripts/build-ios.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."   # airledger repo root

build_for() {
  local target=$1
  echo "==> $target"
  cargo rustc --release \
    --target "$target" --crate-type staticlib
}

build_for aarch64-apple-ios
build_for aarch64-apple-ios-sim
build_for x86_64-apple-ios

out=sdk-dart/build/AirledgerEngine.xcframework
rm -rf "$out"

# Combine sim slices into one lib via lipo, then bundle as xcframework.
mkdir -p sdk-dart/build/sim
lipo -create \
  target/aarch64-apple-ios-sim/release/libairledger_engine.a \
  target/x86_64-apple-ios/release/libairledger_engine.a \
  -output sdk-dart/build/sim/libairledger_engine.a

xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libairledger_engine.a \
  -library sdk-dart/build/sim/libairledger_engine.a \
  -output "$out"

echo
echo "xcframework ready: $out"
echo "in your Flutter iOS app's Runner.xcworkspace, add this as a"
echo "framework dependency under General > Frameworks, Libraries, and Embedded Content."
