/// Raw FFI symbol bindings for `libairledger_engine`. Mirrors
/// airlayer's `sdk-dart/lib/src/bindings.dart`.
///
/// Every exported function is wrapped here once. The higher-level
/// `AirledgerEngine` class in `airledger_engine_base.dart` owns the
/// allocation lifecycle (calloc + free) so callers never see raw
/// pointers.
library;

import 'dart:ffi' as ffi;

typedef _VersionNative = ffi.Pointer<ffi.Char> Function();
typedef _CallNative = ffi.Pointer<ffi.Char> Function(
  ffi.Pointer<ffi.Char>,
);
typedef _PairNative = ffi.Pointer<ffi.Char> Function(
  ffi.Pointer<ffi.Char>,
  ffi.Pointer<ffi.Char>,
);
typedef _FreeNative = ffi.Void Function(ffi.Pointer<ffi.Char>);
typedef _FreeDart = void Function(ffi.Pointer<ffi.Char>);

// --- sheets handle ---
// Opaque pointer to a Rust SheetsHandle. We use `ffi.Void` so the
// Dart side never tries to deref it.
typedef SheetsHandle = ffi.Void;

typedef _SheetsConnectNative = ffi.Pointer<SheetsHandle> Function(
  ffi.Pointer<ffi.Char>,
  ffi.Pointer<ffi.Char>,
  ffi.Pointer<ffi.Pointer<ffi.Char>>,
);
typedef _SheetsFreeHandleNative = ffi.Void Function(
  ffi.Pointer<SheetsHandle>,
);
typedef _SheetsFreeHandleDart = void Function(ffi.Pointer<SheetsHandle>);
typedef _SheetsOpOneNative = ffi.Pointer<ffi.Char> Function(
  ffi.Pointer<SheetsHandle>,
  ffi.Pointer<ffi.Char>,
);
typedef _SheetsListNative = ffi.Pointer<ffi.Char> Function(
  ffi.Pointer<SheetsHandle>,
  ffi.Pointer<ffi.Char>,
  ffi.Pointer<ffi.Char>,
);
typedef _SheetsOpTwoNative = ffi.Pointer<ffi.Char> Function(
  ffi.Pointer<SheetsHandle>,
  ffi.Pointer<ffi.Char>,
  ffi.Pointer<ffi.Char>,
);

class EngineBindings {
  EngineBindings(ffi.DynamicLibrary lib)
      : version = lib
            .lookup<ffi.NativeFunction<_VersionNative>>(
                'airledger_engine_version')
            .asFunction(),
        parseView = lib
            .lookup<ffi.NativeFunction<_CallNative>>(
                'airledger_engine_parse_view')
            .asFunction(),
        parseInputOverlay = lib
            .lookup<ffi.NativeFunction<_CallNative>>(
                'airledger_engine_parse_input_overlay')
            .asFunction(),
        parseViewPair = lib
            .lookup<ffi.NativeFunction<_PairNative>>(
                'airledger_engine_parse_view_pair')
            .asFunction(),
        free = lib
            .lookup<ffi.NativeFunction<_FreeNative>>(
                'airledger_engine_free')
            .asFunction(),
        sheetsConnect = lib
            .lookup<ffi.NativeFunction<_SheetsConnectNative>>(
                'airledger_engine_sheets_connect')
            .asFunction(),
        sheetsFreeHandle = lib
            .lookup<ffi.NativeFunction<_SheetsFreeHandleNative>>(
                'airledger_engine_sheets_free_handle')
            .asFunction(),
        sheetsEnsure = lib
            .lookup<ffi.NativeFunction<_SheetsOpOneNative>>(
                'airledger_engine_sheets_ensure')
            .asFunction(),
        sheetsList = lib
            .lookup<ffi.NativeFunction<_SheetsListNative>>(
                'airledger_engine_sheets_list')
            .asFunction(),
        sheetsCreate = lib
            .lookup<ffi.NativeFunction<_SheetsOpTwoNative>>(
                'airledger_engine_sheets_create')
            .asFunction(),
        sheetsUpdate = lib
            .lookup<ffi.NativeFunction<_SheetsOpTwoNative>>(
                'airledger_engine_sheets_update')
            .asFunction(),
        sheetsDelete = lib
            .lookup<ffi.NativeFunction<_SheetsOpTwoNative>>(
                'airledger_engine_sheets_delete')
            .asFunction(),
        sheetsFreeHandlePtr = lib
            .lookup<ffi.NativeFunction<_SheetsFreeHandleNative>>(
                'airledger_engine_sheets_free_handle');

  final ffi.Pointer<ffi.Char> Function() version;
  final ffi.Pointer<ffi.Char> Function(ffi.Pointer<ffi.Char>) parseView;
  final ffi.Pointer<ffi.Char> Function(ffi.Pointer<ffi.Char>)
      parseInputOverlay;
  final ffi.Pointer<ffi.Char> Function(
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Char>,
  ) parseViewPair;
  final _FreeDart free;

  // sheets handle
  final ffi.Pointer<SheetsHandle> Function(
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Pointer<ffi.Char>>,
  ) sheetsConnect;
  final _SheetsFreeHandleDart sheetsFreeHandle;
  final ffi.Pointer<ffi.Char> Function(
    ffi.Pointer<SheetsHandle>,
    ffi.Pointer<ffi.Char>,
  ) sheetsEnsure;
  final ffi.Pointer<ffi.Char> Function(
    ffi.Pointer<SheetsHandle>,
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Char>,
  ) sheetsList;
  final ffi.Pointer<ffi.Char> Function(
    ffi.Pointer<SheetsHandle>,
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Char>,
  ) sheetsCreate;
  final ffi.Pointer<ffi.Char> Function(
    ffi.Pointer<SheetsHandle>,
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Char>,
  ) sheetsUpdate;
  final ffi.Pointer<ffi.Char> Function(
    ffi.Pointer<SheetsHandle>,
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Char>,
  ) sheetsDelete;

  /// The native function pointer for `sheets_free_handle`, kept around
  /// so `Finalizer` can call it from the GC thread.
  final ffi.Pointer<ffi.NativeFunction<_SheetsFreeHandleNative>>
      sheetsFreeHandlePtr;
}
