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

/// Open a sheets repository backed by the engine. Returned object
/// owns a Rust handle — call [`EngineSheetsRepository.close`] when
/// done (a Finalizer drops it as a safety net if you don't).
extension AirledgerEngineSheets on AirledgerEngine {
  /// Connect to a Google Sheets workbook via the engine's
  /// `SheetsRepository`. Throws [`EngineError`] on bad credentials JSON.
  EngineSheetsRepository connectSheets({
    required String defaultSpreadsheetId,
    required String serviceAccountJson,
  }) {
    final sidPtr = defaultSpreadsheetId.toNativeUtf8().cast<ffi.Char>();
    final saPtr = serviceAccountJson.toNativeUtf8().cast<ffi.Char>();
    final errOut = calloc<ffi.Pointer<ffi.Char>>();
    try {
      final handle = _b.sheetsConnect(sidPtr, saPtr, errOut);
      if (handle == ffi.nullptr) {
        final errPtr = errOut.value;
        final msg = errPtr == ffi.nullptr
            ? 'sheets connect failed (no error message)'
            : errPtr.cast<Utf8>().toDartString();
        if (errPtr != ffi.nullptr) _b.free(errPtr);
        throw EngineError(msg);
      }
      return EngineSheetsRepository._(_b, handle);
    } finally {
      calloc.free(sidPtr);
      calloc.free(saPtr);
      calloc.free(errOut);
    }
  }
}

/// Dart-side wrapper over a Rust `SheetsHandle`. Methods serialize
/// the view + record to JSON, call into the engine, and decode the
/// JSON response into typed Dart values.
///
/// Records use the same tagged envelope the engine emits:
///   `{"kind":"int","value":42}`, `{"kind":"date","value":"2026-06-19"}`,
///   `{"kind":"null"}`, ...
/// Use [`recordToEngineJson`] and [`recordFromEngineJson`] to convert
/// between this wire form and `Map<String, Object?>` records the
/// rest of the app uses.
class EngineSheetsRepository {
  EngineSheetsRepository._(this._b, this._handle) {
    _finalizer.attach(this, _Token(_b, _handle), detach: this);
  }

  final EngineBindings _b;
  ffi.Pointer<SheetsHandle> _handle;
  bool _closed = false;

  static final Finalizer<_Token> _finalizer = Finalizer<_Token>((t) {
    t.bindings.sheetsFreeHandle(t.pointer);
  });

  /// Explicit lifecycle close. Idempotent. Safe to call alongside
  /// the Finalizer — the second call no-ops.
  void close() {
    if (_closed) return;
    _closed = true;
    _finalizer.detach(this);
    _b.sheetsFreeHandle(_handle);
    _handle = ffi.nullptr;
  }

  /// `ensure_sheet` — create the tab if missing, additively merge
  /// the view's headers.
  void ensureSheet(Map<String, dynamic> viewJson) {
    _callOne(_b.sheetsEnsure, viewJson);
  }

  /// `list` — return every data row. Optional [`onDate`] filters
  /// to rows whose `date_field` falls on that day. Records are
  /// JSON in the tagged-envelope wire form (use
  /// [`recordFromEngineJson`] to convert to native Dart values).
  List<Map<String, dynamic>> list(
    Map<String, dynamic> viewJson, {
    DateTime? onDate,
  }) {
    _ensureOpen();
    final viewPtr = jsonEncode(viewJson).toNativeUtf8().cast<ffi.Char>();
    final datePtr = onDate == null
        ? ffi.nullptr.cast<ffi.Char>()
        : _isoDate(onDate).toNativeUtf8().cast<ffi.Char>();
    try {
      final out = _b.sheetsList(_handle, viewPtr, datePtr);
      final decoded = _decode(out);
      if (decoded is! List) {
        throw EngineError('list expected array, got $decoded');
      }
      return decoded.cast<Map<String, dynamic>>();
    } finally {
      calloc.free(viewPtr);
      if (datePtr != ffi.nullptr.cast<ffi.Char>()) calloc.free(datePtr);
    }
  }

  /// `create` — insert at sheet row 2 (newest-first). Returns the
  /// inserted record with `__row = 0` and any auto-assigned `id`.
  Map<String, dynamic> create(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) {
    return _callTwo(_b.sheetsCreate, viewJson, recordJson);
  }

  /// `update` — resolve row by `__row` or `id`, overwrite. Throws
  /// on resolution failure.
  void update(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) {
    _callTwo(_b.sheetsUpdate, viewJson, recordJson);
  }

  /// `delete` — resolve row by `__row` or `id`, drop. Silently
  /// no-ops if the row can't be resolved.
  void delete(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) {
    _callTwo(_b.sheetsDelete, viewJson, recordJson);
  }

  // ---------------------------------------------------------- internals

  void _ensureOpen() {
    if (_closed) {
      throw StateError('EngineSheetsRepository used after close()');
    }
  }

  Object? _callOne(
    ffi.Pointer<ffi.Char> Function(ffi.Pointer<SheetsHandle>, ffi.Pointer<ffi.Char>) fn,
    Map<String, dynamic> viewJson,
  ) {
    _ensureOpen();
    final viewPtr = jsonEncode(viewJson).toNativeUtf8().cast<ffi.Char>();
    try {
      return _decode(fn(_handle, viewPtr));
    } finally {
      calloc.free(viewPtr);
    }
  }

  Map<String, dynamic> _callTwo(
    ffi.Pointer<ffi.Char> Function(
      ffi.Pointer<SheetsHandle>,
      ffi.Pointer<ffi.Char>,
      ffi.Pointer<ffi.Char>,
    ) fn,
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) {
    _ensureOpen();
    final viewPtr = jsonEncode(viewJson).toNativeUtf8().cast<ffi.Char>();
    final recPtr = jsonEncode(recordJson).toNativeUtf8().cast<ffi.Char>();
    try {
      final decoded = _decode(fn(_handle, viewPtr, recPtr));
      if (decoded is! Map<String, dynamic>) {
        throw EngineError('expected JSON object, got: $decoded');
      }
      return decoded;
    } finally {
      calloc.free(viewPtr);
      calloc.free(recPtr);
    }
  }

  Object? _decode(ffi.Pointer<ffi.Char> ptr) {
    final s = ptr.cast<Utf8>().toDartString();
    _b.free(ptr);
    final decoded = jsonDecode(s);
    if (decoded is Map && decoded['error'] is String) {
      throw EngineError(decoded['error'] as String);
    }
    return decoded;
  }

  String _isoDate(DateTime d) {
    final y = d.year.toString().padLeft(4, '0');
    final m = d.month.toString().padLeft(2, '0');
    final dd = d.day.toString().padLeft(2, '0');
    return '$y-$m-$dd';
  }
}

class _Token {
  _Token(this.bindings, this.pointer);
  final EngineBindings bindings;
  final ffi.Pointer<SheetsHandle> pointer;
}

/// Convert a Dart record (`Map<String, Object?>` with native
/// bool/num/String/DateTime/null values) into the tagged-envelope
/// JSON the engine accepts on `create` / `update` / `delete`.
Map<String, dynamic> recordToEngineJson(Map<String, Object?> record) {
  final out = <String, dynamic>{};
  for (final entry in record.entries) {
    out[entry.key] = _valueToTagged(entry.value);
  }
  return out;
}

/// Inverse of [`recordToEngineJson`] — converts a single engine
/// record into native Dart values.
Map<String, Object?> recordFromEngineJson(Map<String, dynamic> json) {
  final out = <String, Object?>{};
  for (final entry in json.entries) {
    out[entry.key] = _valueFromTagged(entry.value);
  }
  return out;
}

Map<String, dynamic> _valueToTagged(Object? v) {
  if (v == null) return const {'kind': 'null'};
  if (v is bool) return {'kind': 'bool', 'value': v};
  if (v is int) return {'kind': 'int', 'value': v};
  if (v is double) return {'kind': 'float', 'value': v};
  if (v is num) return {'kind': 'float', 'value': v.toDouble()};
  if (v is DateTime) {
    // Calendar-only DateTimes (midnight, hour/minute/second all zero)
    // go as Date; otherwise DateTime. Matches the codec's behavior on
    // the Rust side.
    if (v.hour == 0 && v.minute == 0 && v.second == 0 && v.millisecond == 0) {
      return {
        'kind': 'date',
        'value': '${v.year.toString().padLeft(4, '0')}-'
            '${v.month.toString().padLeft(2, '0')}-'
            '${v.day.toString().padLeft(2, '0')}',
      };
    }
    return {
      'kind': 'date_time',
      'value': v.toIso8601String().split('.').first,
    };
  }
  return {'kind': 'string', 'value': v.toString()};
}

Object? _valueFromTagged(Object? tagged) {
  if (tagged is! Map) return null;
  switch (tagged['kind']) {
    case 'null':
      return null;
    case 'bool':
      return tagged['value'] as bool;
    case 'int':
      return tagged['value'] as int;
    case 'float':
      return (tagged['value'] as num).toDouble();
    case 'string':
      return tagged['value'] as String;
    case 'date':
      return DateTime.parse(tagged['value'] as String);
    case 'date_time':
      return DateTime.parse(tagged['value'] as String);
  }
  return null;
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
