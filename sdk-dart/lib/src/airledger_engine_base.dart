import 'dart:convert';
import 'dart:ffi' as ffi;
import 'dart:io';
import 'dart:isolate';

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
  Future<void> ensureSheet(Map<String, dynamic> viewJson) async {
    _ensureOpen();
    final addr = _handle.address;
    final view = jsonEncode(viewJson);
    await Isolate.run(() => _runOne(_OpKind.ensure, addr, view, null));
  }

  /// `list` — return every data row. Optional [`onDate`] filters
  /// to rows whose `date_field` falls on that day. Records are
  /// JSON in the tagged-envelope wire form (use
  /// [`recordFromEngineJson`] to convert to native Dart values).
  Future<List<Map<String, dynamic>>> list(
    Map<String, dynamic> viewJson, {
    DateTime? onDate,
  }) async {
    _ensureOpen();
    final addr = _handle.address;
    final view = jsonEncode(viewJson);
    final dateStr = onDate == null ? null : _isoDate(onDate);
    final decoded =
        await Isolate.run(() => _runList(addr, view, dateStr));
    if (decoded is! List) {
      throw EngineError('list expected array, got $decoded');
    }
    return decoded.cast<Map<String, dynamic>>();
  }

  /// `create` — insert at sheet row 2 (newest-first). Returns the
  /// inserted record with `__row = 0` and any auto-assigned `id`.
  Future<Map<String, dynamic>> create(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    return _runTwoAsObject(_OpKind.create, viewJson, recordJson);
  }

  /// `update` — resolve row by `__row` or `id`, overwrite. Throws
  /// on resolution failure.
  Future<void> update(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    await _runTwoAsObject(_OpKind.update, viewJson, recordJson);
  }

  /// `delete` — resolve row by `__row` or `id`, drop. Silently
  /// no-ops if the row can't be resolved.
  Future<void> delete(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    await _runTwoAsObject(_OpKind.delete, viewJson, recordJson);
  }

  // ---------------------------------------------------------- internals

  void _ensureOpen() {
    if (_closed) {
      throw StateError('EngineSheetsRepository used after close()');
    }
  }

  Future<Map<String, dynamic>> _runTwoAsObject(
    _OpKind kind,
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    _ensureOpen();
    final addr = _handle.address;
    final view = jsonEncode(viewJson);
    final record = jsonEncode(recordJson);
    final decoded = await Isolate.run(() => _runTwo(kind, addr, view, record));
    if (decoded is! Map<String, dynamic>) {
      throw EngineError('expected JSON object, got: $decoded');
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

/// Open a local-first ledger backed by the engine: SQLite store as
/// source of truth, Sheets as the sync target. CRUD never touches
/// the network; only [EngineLedgerRepository.sync] does.
extension AirledgerEngineLedger on AirledgerEngine {
  /// Throws [EngineError] when the DB can't be opened or the service
  /// account JSON is malformed (credentials are parsed, not
  /// exercised — opening works offline).
  EngineLedgerRepository openLedger({
    required String dbPath,
    required String defaultSpreadsheetId,
    required String serviceAccountJson,
  }) {
    final dbPtr = dbPath.toNativeUtf8().cast<ffi.Char>();
    final sidPtr = defaultSpreadsheetId.toNativeUtf8().cast<ffi.Char>();
    final saPtr = serviceAccountJson.toNativeUtf8().cast<ffi.Char>();
    final errOut = calloc<ffi.Pointer<ffi.Char>>();
    try {
      final handle = _b.ledgerOpen(dbPtr, sidPtr, saPtr, errOut);
      if (handle == ffi.nullptr) {
        final errPtr = errOut.value;
        final msg = errPtr == ffi.nullptr
            ? 'ledger open failed (no error message)'
            : errPtr.cast<Utf8>().toDartString();
        if (errPtr != ffi.nullptr) _b.free(errPtr);
        throw EngineError(msg);
      }
      return EngineLedgerRepository._(_b, handle);
    } finally {
      calloc.free(dbPtr);
      calloc.free(sidPtr);
      calloc.free(saPtr);
      calloc.free(errOut);
    }
  }
}

/// Dart-side wrapper over a Rust `LedgerHandle`. Same record wire
/// form as [EngineSheetsRepository] (tagged envelopes; use
/// [recordToEngineJson] / [recordFromEngineJson]).
class EngineLedgerRepository {
  EngineLedgerRepository._(this._b, this._handle) {
    _finalizer.attach(this, _LedgerToken(_b, _handle), detach: this);
  }

  final EngineBindings _b;
  ffi.Pointer<LedgerHandle> _handle;
  bool _closed = false;

  static final Finalizer<_LedgerToken> _finalizer =
      Finalizer<_LedgerToken>((t) {
    t.bindings.ledgerFreeHandle(t.pointer);
  });

  /// Explicit lifecycle close. Idempotent.
  void close() {
    if (_closed) return;
    _closed = true;
    _finalizer.detach(this);
    _b.ledgerFreeHandle(_handle);
    _handle = ffi.nullptr;
  }

  /// Local `list` — instant, zero network. Optional [onDate] filters
  /// by the view's `date_field`.
  Future<List<Map<String, dynamic>>> list(
    Map<String, dynamic> viewJson, {
    DateTime? onDate,
  }) async {
    _ensureOpen();
    final addr = _handle.address;
    final view = jsonEncode(viewJson);
    final dateStr = onDate == null ? null : _isoDateOf(onDate);
    final decoded =
        await Isolate.run(() => _ledgerRunList(addr, view, dateStr));
    if (decoded is! List) {
      throw EngineError('list expected array, got $decoded');
    }
    return decoded.cast<Map<String, dynamic>>();
  }

  /// Local `create`. Returns the stored record with any
  /// auto-assigned `id`.
  Future<Map<String, dynamic>> create(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    return _runTwo(_LedgerOpKind.create, viewJson, recordJson);
  }

  /// Local `update` — addresses by `id`.
  Future<void> update(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    await _runTwo(_LedgerOpKind.update, viewJson, recordJson);
  }

  /// Local `delete` — tombstones synced rows, removes unsynced ones.
  Future<void> delete(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    await _runTwo(_LedgerOpKind.delete, viewJson, recordJson);
  }

  /// Count of local changes not yet pushed to the Sheet.
  Future<int> pending() async {
    _ensureOpen();
    final addr = _handle.address;
    final decoded = await Isolate.run(() => _ledgerRunPending(addr));
    if (decoded is! Map || decoded['pending'] is! int) {
      throw EngineError('pending expected {pending: n}, got $decoded');
    }
    return decoded['pending'] as int;
  }

  /// Run a full sync for [views] (JSON-decoded ViewSchemas). Network
  /// happens here and only here. Returns per-view result maps
  /// (`{view, pulled, pushed, deleted_local, deleted_remote,
  /// conflicts, error}`); a view's failure lands in its `error`
  /// field rather than throwing.
  Future<List<Map<String, dynamic>>> sync(
    List<Map<String, dynamic>> views,
  ) async {
    _ensureOpen();
    final addr = _handle.address;
    final viewsJson = jsonEncode(views);
    final decoded = await Isolate.run(() => _ledgerRunSync(addr, viewsJson));
    if (decoded is! List) {
      throw EngineError('sync expected array, got $decoded');
    }
    return decoded.cast<Map<String, dynamic>>();
  }

  /// Merge an externally-sourced batch (see the engine's
  /// `IngestBatch`: source, owned_fields, fill_if_blank_fields,
  /// records, deleted_dates). Returns
  /// `{created, updated, unchanged, skipped, deleted, cleared}`.
  Future<Map<String, dynamic>> ingest(
    Map<String, dynamic> viewJson,
    Map<String, dynamic> batchJson,
  ) async {
    _ensureOpen();
    final addr = _handle.address;
    final view = jsonEncode(viewJson);
    final batch = jsonEncode(batchJson);
    final decoded =
        await Isolate.run(() => _ledgerRunIngest(addr, view, batch));
    if (decoded is! Map<String, dynamic>) {
      throw EngineError('ingest expected JSON object, got: $decoded');
    }
    return decoded;
  }

  /// Small per-ledger key/value store (integration cursors, status).
  Future<String?> metaGet(String key) async {
    _ensureOpen();
    final addr = _handle.address;
    final decoded = await Isolate.run(() => _ledgerRunMetaGet(addr, key));
    if (decoded is! Map) {
      throw EngineError('metaGet expected object, got: $decoded');
    }
    return decoded['value'] as String?;
  }

  Future<void> metaSet(String key, String value) async {
    _ensureOpen();
    final addr = _handle.address;
    await Isolate.run(() => _ledgerRunMetaSet(addr, key, value));
  }

  // ---------------------------------------------------------- internals

  void _ensureOpen() {
    if (_closed) {
      throw StateError('EngineLedgerRepository used after close()');
    }
  }

  Future<Map<String, dynamic>> _runTwo(
    _LedgerOpKind kind,
    Map<String, dynamic> viewJson,
    Map<String, dynamic> recordJson,
  ) async {
    _ensureOpen();
    final addr = _handle.address;
    final view = jsonEncode(viewJson);
    final record = jsonEncode(recordJson);
    final decoded =
        await Isolate.run(() => _ledgerRunTwo(kind, addr, view, record));
    if (decoded is! Map<String, dynamic>) {
      throw EngineError('expected JSON object, got: $decoded');
    }
    return decoded;
  }

  String _isoDateOf(DateTime d) {
    final y = d.year.toString().padLeft(4, '0');
    final m = d.month.toString().padLeft(2, '0');
    final dd = d.day.toString().padLeft(2, '0');
    return '$y-$m-$dd';
  }
}

class _LedgerToken {
  _LedgerToken(this.bindings, this.pointer);
  final EngineBindings bindings;
  final ffi.Pointer<LedgerHandle> pointer;
}

enum _LedgerOpKind { create, update, delete }

Object? _ledgerRunList(int handleAddr, String viewJson, String? dateStr) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<LedgerHandle>.fromAddress(handleAddr);
  final viewPtr = viewJson.toNativeUtf8().cast<ffi.Char>();
  final datePtr = dateStr == null
      ? ffi.nullptr.cast<ffi.Char>()
      : dateStr.toNativeUtf8().cast<ffi.Char>();
  try {
    return _decodePtr(b, b.ledgerList(handle, viewPtr, datePtr));
  } finally {
    calloc.free(viewPtr);
    if (datePtr != ffi.nullptr.cast<ffi.Char>()) calloc.free(datePtr);
  }
}

Object? _ledgerRunTwo(
  _LedgerOpKind kind,
  int handleAddr,
  String viewJson,
  String recordJson,
) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<LedgerHandle>.fromAddress(handleAddr);
  final viewPtr = viewJson.toNativeUtf8().cast<ffi.Char>();
  final recPtr = recordJson.toNativeUtf8().cast<ffi.Char>();
  try {
    final fn = switch (kind) {
      _LedgerOpKind.create => b.ledgerCreate,
      _LedgerOpKind.update => b.ledgerUpdate,
      _LedgerOpKind.delete => b.ledgerDelete,
    };
    return _decodePtr(b, fn(handle, viewPtr, recPtr));
  } finally {
    calloc.free(viewPtr);
    calloc.free(recPtr);
  }
}

Object? _ledgerRunIngest(int handleAddr, String viewJson, String batchJson) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<LedgerHandle>.fromAddress(handleAddr);
  final viewPtr = viewJson.toNativeUtf8().cast<ffi.Char>();
  final batchPtr = batchJson.toNativeUtf8().cast<ffi.Char>();
  try {
    return _decodePtr(b, b.ledgerIngest(handle, viewPtr, batchPtr));
  } finally {
    calloc.free(viewPtr);
    calloc.free(batchPtr);
  }
}

Object? _ledgerRunMetaGet(int handleAddr, String key) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<LedgerHandle>.fromAddress(handleAddr);
  final keyPtr = key.toNativeUtf8().cast<ffi.Char>();
  try {
    return _decodePtr(b, b.ledgerMetaGet(handle, keyPtr));
  } finally {
    calloc.free(keyPtr);
  }
}

Object? _ledgerRunMetaSet(int handleAddr, String key, String value) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<LedgerHandle>.fromAddress(handleAddr);
  final keyPtr = key.toNativeUtf8().cast<ffi.Char>();
  final valPtr = value.toNativeUtf8().cast<ffi.Char>();
  try {
    return _decodePtr(b, b.ledgerMetaSet(handle, keyPtr, valPtr));
  } finally {
    calloc.free(keyPtr);
    calloc.free(valPtr);
  }
}

Object? _ledgerRunPending(int handleAddr) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<LedgerHandle>.fromAddress(handleAddr);
  return _decodePtr(b, b.ledgerPending(handle));
}

Object? _ledgerRunSync(int handleAddr, String viewsJson) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<LedgerHandle>.fromAddress(handleAddr);
  final viewsPtr = viewsJson.toNativeUtf8().cast<ffi.Char>();
  try {
    return _decodePtr(b, b.ledgerSync(handle, viewsPtr));
  } finally {
    calloc.free(viewsPtr);
  }
}

/// Op kind tag for the worker isolate dispatcher. Plain `int` so it
/// crosses the isolate boundary cheaply.
enum _OpKind { ensure, create, update, delete }

/// Worker entry for `ensure_sheet` (one-arg FFI). Runs inside an
/// `Isolate.run` closure, so it must be top-level / static and may
/// only capture sendable values.
Object? _runOne(_OpKind kind, int handleAddr, String viewJson, String? _) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<SheetsHandle>.fromAddress(handleAddr);
  final viewPtr = viewJson.toNativeUtf8().cast<ffi.Char>();
  try {
    final fn = _selectOne(b, kind);
    return _decodePtr(b, fn(handle, viewPtr));
  } finally {
    calloc.free(viewPtr);
  }
}

Object? _runList(int handleAddr, String viewJson, String? dateStr) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<SheetsHandle>.fromAddress(handleAddr);
  final viewPtr = viewJson.toNativeUtf8().cast<ffi.Char>();
  final datePtr = dateStr == null
      ? ffi.nullptr.cast<ffi.Char>()
      : dateStr.toNativeUtf8().cast<ffi.Char>();
  try {
    return _decodePtr(b, b.sheetsList(handle, viewPtr, datePtr));
  } finally {
    calloc.free(viewPtr);
    if (datePtr != ffi.nullptr.cast<ffi.Char>()) calloc.free(datePtr);
  }
}

Object? _runTwo(
  _OpKind kind,
  int handleAddr,
  String viewJson,
  String recordJson,
) {
  final b = _bindingsForCurrentIsolate();
  final handle = ffi.Pointer<SheetsHandle>.fromAddress(handleAddr);
  final viewPtr = viewJson.toNativeUtf8().cast<ffi.Char>();
  final recPtr = recordJson.toNativeUtf8().cast<ffi.Char>();
  try {
    final fn = _selectTwo(b, kind);
    return _decodePtr(b, fn(handle, viewPtr, recPtr));
  } finally {
    calloc.free(viewPtr);
    calloc.free(recPtr);
  }
}

ffi.Pointer<ffi.Char> Function(
  ffi.Pointer<SheetsHandle>,
  ffi.Pointer<ffi.Char>,
) _selectOne(EngineBindings b, _OpKind kind) {
  switch (kind) {
    case _OpKind.ensure:
      return b.sheetsEnsure;
    case _OpKind.create:
    case _OpKind.update:
    case _OpKind.delete:
      throw StateError('not a one-arg op: $kind');
  }
}

ffi.Pointer<ffi.Char> Function(
  ffi.Pointer<SheetsHandle>,
  ffi.Pointer<ffi.Char>,
  ffi.Pointer<ffi.Char>,
) _selectTwo(EngineBindings b, _OpKind kind) {
  switch (kind) {
    case _OpKind.create:
      return b.sheetsCreate;
    case _OpKind.update:
      return b.sheetsUpdate;
    case _OpKind.delete:
      return b.sheetsDelete;
    case _OpKind.ensure:
      throw StateError('not a two-arg op: $kind');
  }
}

Object? _decodePtr(EngineBindings b, ffi.Pointer<ffi.Char> ptr) {
  final s = ptr.cast<Utf8>().toDartString();
  b.free(ptr);
  final decoded = jsonDecode(s);
  if (decoded is Map && decoded['error'] is String) {
    throw EngineError(decoded['error'] as String);
  }
  return decoded;
}

EngineBindings? _isolateBindings;

/// Lazily build a bindings instance for the current isolate. Each
/// worker isolate looks up the library + symbols once, then caches
/// for any subsequent calls in the same isolate. With `Isolate.run`
/// this still means one lookup per call (each call spawns a fresh
/// isolate) — but the dynamic linker keeps the .so loaded process-
/// wide, so the lookup is cheap.
EngineBindings _bindingsForCurrentIsolate() {
  return _isolateBindings ??= EngineBindings(_openLibrary());
}

ffi.DynamicLibrary _openLibrary() {
  if (Platform.isAndroid) {
    return ffi.DynamicLibrary.open('libairledger_engine.so');
  }
  if (Platform.isIOS) {
    return ffi.DynamicLibrary.process();
  }
  // Host platforms reuse `AirledgerEngine.load`'s candidate search.
  return AirledgerEngine.load()._b == _isolateBindings
      ? ffi.DynamicLibrary.process()
      : (() {
          // Fall back to the same loader the main isolate uses.
          // Tests run on the host and need the cargo target/ paths;
          // duplicate the logic here so workers don't depend on
          // main-isolate state.
          for (final p in _hostCandidatePaths()) {
            if (File(p).existsSync()) return ffi.DynamicLibrary.open(p);
          }
          return ffi.DynamicLibrary.process();
        })();
}

List<String> _hostCandidatePaths() {
  final cwd = Directory.current.path;
  final name = Platform.isMacOS
      ? 'libairledger_engine.dylib'
      : Platform.isWindows
          ? 'airledger_engine.dll'
          : 'libairledger_engine.so';
  return [
    '$cwd/target/release/$name',
    '$cwd/target/debug/$name',
    '$cwd/../target/release/$name',
    '$cwd/../target/debug/$name',
    name,
  ];
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
