import 'dart:io';

import 'package:airledger_engine/airledger_engine.dart';
import 'package:test/test.dart';

// Structure-only SA json — the PEM is validated lazily at token
// time, which offline CRUD never reaches.
const fakeSa = '''
{"type":"service_account","project_id":"t","private_key_id":"k1",
 "private_key":"-----BEGIN PRIVATE KEY-----\\nZmFrZQ==\\n-----END PRIVATE KEY-----\\n",
 "client_email":"t@t.iam.gserviceaccount.com","client_id":"1",
 "token_uri":"https://oauth2.googleapis.com/token"}
''';

const viewYaml = '''
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: weight_lbs, type: number, expr: weight_lbs }
''';

void main() {
  test('ledger CRUD works offline and tracks pending', () async {
    final engine = AirledgerEngine.load();
    final dir = Directory.systemTemp.createTempSync('ledger');
    final ledger = engine.openLedger(
      dbPath: '${dir.path}/t.db',
      defaultSpreadsheetId: 'unused',
      serviceAccountJson: fakeSa,
    );
    final view = engine.parseView(viewYaml);

    final created =
        await ledger.create(view, recordToEngineJson({'weight_lbs': 180.5}));
    final createdNative = recordFromEngineJson(created);
    expect(createdNative['id'], isA<String>());
    expect((createdNative['id'] as String), isNotEmpty);

    final rows = await ledger.list(view);
    expect(rows, hasLength(1));
    expect(recordFromEngineJson(rows.first)['weight_lbs'], 180.5);
    expect(await ledger.pending(), 1);

    // Update round-trips.
    final edited = Map<String, dynamic>.from(created);
    edited['weight_lbs'] = {'kind': 'float', 'value': 179.0};
    await ledger.update(view, edited);
    final after = await ledger.list(view);
    expect(recordFromEngineJson(after.first)['weight_lbs'], 179.0);

    // Delete of a never-synced row removes outright.
    await ledger.delete(view, created);
    expect(await ledger.list(view), isEmpty);
    expect(await ledger.pending(), 0);

    ledger.close();
    dir.deleteSync(recursive: true);
  });

  test('ingest merges a batch and meta round-trips', () async {
    final engine = AirledgerEngine.load();
    final dir = Directory.systemTemp.createTempSync('ingest');
    final ledger = engine.openLedger(
      dbPath: '${dir.path}/t.db',
      defaultSpreadsheetId: 'unused',
      serviceAccountJson: fakeSa,
    );
    final view = engine.parseViewPair(viewYaml: '''
name: weight
datasource: gsheets
table: weight
dimensions:
  - { name: id, type: string, expr: id }
  - { name: date, type: date, expr: date }
  - { name: body_fat_withing, type: number, expr: body_fat_withing }
''', inputYaml: '''
target: weight.view.yml
date_field: date
''');
    final res = await ledger.ingest(view, {
      'source': 'withings',
      'owned_fields': ['body_fat_withing'],
      'records': [
        {
          'date': {'kind': 'date', 'value': '2026-08-28'},
          'body_fat_withing': {'kind': 'float', 'value': 18.2},
        }
      ],
    });
    expect(res['created'], 1);
    expect(await ledger.list(view), hasLength(1));

    await ledger.metaSet('integration_cursor_withings', '123');
    expect(await ledger.metaGet('integration_cursor_withings'), '123');
    expect(await ledger.metaGet('never_written'), isNull);
    ledger.close();
    dir.deleteSync(recursive: true);
  });

  test('openLedger surfaces engine errors', () {
    final engine = AirledgerEngine.load();
    expect(
      () => engine.openLedger(
        dbPath: '/nonexistent-dir-xyz/t.db',
        defaultSpreadsheetId: 'unused',
        serviceAccountJson: fakeSa,
      ),
      throwsA(isA<EngineError>()),
    );
  });
}
