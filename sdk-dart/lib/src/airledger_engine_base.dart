import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:ffi/ffi.dart';

import 'bindings.dart';

/// Public entry point for the airledger-engine Dart SDK. Construct
/// via [`AirledgerEngine.load`] which locates the right native
/// library for the current platform.
///
/// Mirrors the shape of airlayer's `Airlayer` class — same lifecycle,
/// same string-in / JSON-out contract.
class AirledgerEngine {
  AirledgerEngine._(this._b);

  final EngineBindings _b;

  /// Load the native library and return a wrapper. Platform search:
  /// - macOS: `libairledger_engine.dylib` next to the executable, then
  ///   the Cargo `target/debug` and `target/release` directories
  ///   relative to this file (for tests / dev).
  /// - Linux: `libairledger_engine.so` with the same search list.
  /// - Windows: `airledger_engine.dll` (untested — added when needed).
  ///
  /// Throws [`StateError`] if no library is found.
  static AirledgerEngine load() {
    final lib = _loadLibrary();
    return AirledgerEngine._(EngineBindings(lib));
  }

  /// Stable version string the engine reports — useful as a smoke
  /// test that the FFI plumbing is wired correctly.
  String get version {
    final ptr = _b.version();
    final s = ptr.cast<Utf8>().toDartString();
    _b.free(ptr);
    return s;
  }

  /// Parse a `.view.yml` document. Returns the JSON-decoded
  /// [`ViewSchema`] map on success. Throws [`EngineError`] on parse
  /// failure (the Rust side returns a structured `{"error": "..."}`
  /// blob that this wrapper unpacks).
  Map<String, dynamic> parseView(String yaml) {
    return _call(_b.parseView, yaml);
  }

  /// Parse a `.input.yml` document. Returns the JSON-decoded
  /// `InputOverlay` map.
  Map<String, dynamic> parseInputOverlay(String yaml) {
    return _call(_b.parseInputOverlay, yaml);
  }

  /// Parse both files and merge in one round-trip. Saves a parse →
  /// parse → merge dance on the Dart side and means the caller gets a
  /// fully-resolved `ViewSchema` (with the input overlay already
  /// applied) back in one call.
  Map<String, dynamic> parseViewPair({
    required String viewYaml,
    required String inputYaml,
  }) {
    final aPtr = viewYaml.toNativeUtf8().cast<ffi.Char>();
    final bPtr = inputYaml.toNativeUtf8().cast<ffi.Char>();
    try {
      final out = _b.parseViewPair(aPtr, bPtr);
      return _decode(out);
    } finally {
      calloc.free(aPtr);
      calloc.free(bPtr);
    }
  }

  // ----------------------------------------------------------- helpers

  Map<String, dynamic> _call(
    ffi.Pointer<ffi.Char> Function(ffi.Pointer<ffi.Char>) fn,
    String yaml,
  ) {
    final inPtr = yaml.toNativeUtf8().cast<ffi.Char>();
    try {
      final out = fn(inPtr);
      return _decode(out);
    } finally {
      calloc.free(inPtr);
    }
  }

  Map<String, dynamic> _decode(ffi.Pointer<ffi.Char> ptr) {
    final s = ptr.cast<Utf8>().toDartString();
    _b.free(ptr);
    final decoded = jsonDecode(s);
    if (decoded is! Map<String, dynamic>) {
      throw EngineError('expected JSON object, got: $s');
    }
    if (decoded['error'] is String) {
      throw EngineError(decoded['error'] as String);
    }
    return decoded;
  }
}

class EngineError implements Exception {
  EngineError(this.message);
  final String message;
  @override
  String toString() => 'EngineError: $message';
}

ffi.DynamicLibrary _loadLibrary() {
  // On Android the system dynamic linker resolves the bare filename
  // against the app's nativeLibraryDir (where jniLibs/<abi>/ files
  // land after install) — no dev-path search needed.
  if (Platform.isAndroid) {
    return ffi.DynamicLibrary.open('libairledger_engine.so');
  }
  // On iOS the engine is statically linked into the app binary, so
  // its symbols are already in the process image. No file to open.
  if (Platform.isIOS) {
    return ffi.DynamicLibrary.process();
  }

  final candidates = <String>[];
  if (Platform.isMacOS) {
    candidates.addAll(_candidatePaths('libairledger_engine.dylib'));
  } else if (Platform.isLinux) {
    candidates.addAll(_candidatePaths('libairledger_engine.so'));
  } else if (Platform.isWindows) {
    candidates.addAll(_candidatePaths('airledger_engine.dll'));
  } else {
    throw StateError('Unsupported platform: ${Platform.operatingSystem}');
  }
  for (final path in candidates) {
    if (File(path).existsSync()) {
      return ffi.DynamicLibrary.open(path);
    }
  }
  // Last-ditch: let the dynamic loader search system paths.
  try {
    return ffi.DynamicLibrary.process();
  } catch (_) {/* fall through */}
  throw StateError(
    'libairledger_engine not found. Run `cargo build` in the engine '
    'repo first. Searched:\n  ${candidates.join("\n  ")}',
  );
}

/// Search paths covering the common dev + Flutter-bundled cases.
/// Order matches the priority the caller likely wants: explicit
/// install path → Cargo target dir (for tests / dev) → Flutter
/// bundled location.
List<String> _candidatePaths(String filename) {
  final cwd = Directory.current.path;
  return [
    // Cargo dev build (release first so prod tests pick it).
    '$cwd/target/release/$filename',
    '$cwd/target/debug/$filename',
    // When sdk-dart is the cwd, the engine lives one level up.
    '$cwd/../target/release/$filename',
    '$cwd/../target/debug/$filename',
    // Bare filename — system dynamic-loader path.
    filename,
  ];
}
