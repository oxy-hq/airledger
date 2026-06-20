/// End-to-end FFI smoke test. Loads the host-build dylib and exercises
/// every C-ABI function exposed by `src/ffi.rs` with a real YAML
/// fixture from `tests/fixtures/`. Catches issues in the
/// pointer-passing, string ownership, and JSON contract before the
/// engine grows any more surface.

import 'dart:io';

import 'package:airledger_engine/airledger_engine.dart';
import 'package:test/test.dart';

void main() {
  late AirledgerEngine engine;

  setUpAll(() {
    engine = AirledgerEngine.load();
  });

  test('version reports the engine identity string', () {
    final v = engine.version;
    expect(v, startsWith('airledger-engine '));
  });

  test('parseView round-trips strength.view.yml', () {
    final yaml = _fixture('fitness/strength.view.yml');
    final view = engine.parseView(yaml);
    expect(view['name'], 'strength');
    expect(view['datasource'], 'gsheets');
    expect(view['table'], 'strength');
    expect(view['dimensions'], isA<List>());
    final dims = view['dimensions'] as List;
    expect(
      dims.any((d) => (d as Map)['name'] == 'exercise'),
      isTrue,
      reason: 'strength.exercise dim should round-trip through FFI',
    );
  });

  test('parseInputOverlay round-trips strength.input.yml', () {
    final yaml = _fixture('fitness/strength.input.yml');
    final overlay = engine.parseInputOverlay(yaml);
    expect(overlay['view_name'], 'strength');
    expect(overlay['date_field'], 'date');
    expect(overlay['top_metric'], 'max_e1rm');
    // Groups must include the mobility / timed / isometric value sets
    // we added on the strength schema.
    final groups = overlay['groups'] as Map;
    expect(groups.keys.toList(), containsAll(['mobility', 'timed', 'isometric']));
  });

  test('parseViewPair merges strength view + input overlay', () {
    final view = _fixture('fitness/strength.view.yml');
    final input = _fixture('fitness/strength.input.yml');
    final merged = engine.parseViewPair(viewYaml: view, inputYaml: input);
    expect(merged['name'], 'strength');
    expect(merged['has_input_overlay'], isTrue);
    expect(merged['top_metric'], 'max_e1rm');

    // start_time should have its widget = timer + stop_targets after
    // overlay merge.
    final dims = (merged['dimensions'] as List).cast<Map>();
    final startTime = dims.firstWhere((d) => d['name'] == 'start_time');
    expect((startTime['input'] as Map)['widget'], 'timer');
    final stops = (startTime['input'] as Map)['stop_targets'] as List;
    expect(stops, hasLength(2));
    expect(stops.any((s) => (s as Map)['target'] == 'end_time'), isTrue);
    expect(stops.any((s) => (s as Map)['target'] == 'duration'), isTrue);
  });

  test('parseView returns a structured error for malformed YAML', () {
    expect(
      () => engine.parseView('this is: { broken'),
      throwsA(isA<EngineError>()),
    );
  });

  test('recordToEngineJson + recordFromEngineJson round-trip', () {
    final original = <String, Object?>{
      'id': 'abc-123',
      'note': 'hi',
      'count': 42,
      'weight': 180.5,
      'on_date': DateTime(2026, 6, 19),
      'logged_at': DateTime(2026, 6, 19, 9, 30, 15),
      'empty': null,
      'flag': true,
    };
    final tagged = recordToEngineJson(original);
    // Spot-check the wire shape: dates are tagged, datetimes too.
    expect(tagged['on_date'], {'kind': 'date', 'value': '2026-06-19'});
    expect(tagged['logged_at'], {
      'kind': 'date_time',
      'value': '2026-06-19T09:30:15',
    });
    expect(tagged['empty'], {'kind': 'null'});
    expect(tagged['flag'], {'kind': 'bool', 'value': true});
    expect(tagged['count'], {'kind': 'int', 'value': 42});

    final back = recordFromEngineJson(tagged);
    expect(back['id'], 'abc-123');
    expect(back['note'], 'hi');
    expect(back['count'], 42);
    expect(back['weight'], 180.5);
    expect(back['on_date'], DateTime(2026, 6, 19));
    expect(back['logged_at'], DateTime(2026, 6, 19, 9, 30, 15));
    expect(back['empty'], null);
    expect(back['flag'], true);
  });

  test('connectSheets throws EngineError on bad service-account JSON', () {
    expect(
      () => engine.connectSheets(
        defaultSpreadsheetId: 'test-id',
        serviceAccountJson: '{ not json',
      ),
      throwsA(isA<EngineError>().having(
        (e) => e.message,
        'message',
        contains('service account json'),
      )),
    );
  });

  test('parseViewPair fails when view_name does not match target', () {
    final view = '''
name: foo
datasource: gsheets
table: foo
dimensions:
  - { name: id, type: string, expr: id }
''';
    final input = '''
target: bar.view.yml
fields:
  id: { editable: false }
''';
    expect(
      () => engine.parseViewPair(viewYaml: view, inputYaml: input),
      throwsA(
        isA<EngineError>().having(
          (e) => e.message,
          'message',
          contains('view-name mismatch'),
        ),
      ),
    );
  });
}

/// Read a YAML fixture from the engine crate's tests/fixtures dir.
/// Tests run with `cwd = sdk-dart`; the engine repo's `target/` and
/// `tests/` are one level up.
String _fixture(String relative) {
  final cwd = Directory.current.path;
  final path = '$cwd/../tests/fixtures/$relative';
  return File(path).readAsStringSync();
}
