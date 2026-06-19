/// Dart bindings for the airledger-engine Rust core.
///
/// The engine ships as a single dynamic library
/// (`libairledger_engine.{dylib,so,dll}`) — same playbook as airlayer's
/// `sdk-dart`. This file is the public Dart API; `src/bindings.dart`
/// holds the raw FFI symbols.
library;

export 'src/airledger_engine_base.dart';
