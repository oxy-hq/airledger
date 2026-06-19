#!/usr/bin/env bash
# Build libairledger_engine.so for Android (all common ABIs) for Flutter
# mobile use.
#
# Prereqs:
#   - Android NDK installed (set ANDROID_NDK_HOME or ANDROID_NDK_ROOT)
#   - cargo-ndk:  cargo install cargo-ndk
#   - Rust targets:
#       rustup target add aarch64-linux-android armv7-linux-androideabi \
#                         x86_64-linux-android i686-linux-android
#
# Output: artifacts in target/<triple>/release/libairledger_engine.so and
# copied into sdk-dart/build/jniLibs/<abi>/libairledger_engine.so for
# direct use in a Flutter app's android/app/src/main/jniLibs/ directory.
#
# Mirrors airlayer/sdk-dart/scripts/build-android.sh.
set -euo pipefail
cd "$(dirname "$0")/../.."   # airledger repo root

if ! command -v cargo-ndk >/dev/null; then
  echo "cargo-ndk not installed. install: cargo install cargo-ndk" >&2
  exit 1
fi

cargo ndk \
  -t arm64-v8a \
  -t armeabi-v7a \
  -t x86_64 \
  -t x86 \
  -o sdk-dart/build/jniLibs \
  build --release

echo
echo "android libs ready in sdk-dart/build/jniLibs/"
find sdk-dart/build/jniLibs -name 'libairledger_engine.so' | sed 's/^/  /'
echo
echo "to use in a Flutter app, copy each <abi>/libairledger_engine.so to:"
echo "  <your_app>/android/app/src/main/jniLibs/<abi>/"
