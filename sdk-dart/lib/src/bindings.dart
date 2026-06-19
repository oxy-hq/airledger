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
            .asFunction();

  final ffi.Pointer<ffi.Char> Function() version;
  final ffi.Pointer<ffi.Char> Function(ffi.Pointer<ffi.Char>) parseView;
  final ffi.Pointer<ffi.Char> Function(ffi.Pointer<ffi.Char>)
      parseInputOverlay;
  final ffi.Pointer<ffi.Char> Function(
    ffi.Pointer<ffi.Char>,
    ffi.Pointer<ffi.Char>,
  ) parseViewPair;
  final _FreeDart free;
}
