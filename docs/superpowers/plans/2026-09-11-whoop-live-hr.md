# Whoop Live Heart Rate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Live BLE heart rate from the Whoop band displayed in the cardio form's timer, auto-stamping `zone4_reached`/`zone5_reached` and filling `max_hr` on Stop.

**Architecture:** Two new optional input-schema keys (`ladders[].hr_pct`, `hr_max_target`) flow Rust engine → Dart mirrors → timer widget. A generic `HeartRateService` (BLE Heart Rate Service 0x180D) streams BPM; pure `HrSession` logic decides zone stamps; stamps ride the existing `onLadderTap` path so the normal form save persists everything. A `WhoopIntegration` card handles pairing and the `user_max_hr` meta setting. No ingest/provenance — the user is present in the form.

**Tech Stack:** Rust (serde/serde_yaml), Dart/Flutter, flutter_blue_plus, permission_handler, wakelock_plus.

**Spec:** `docs/superpowers/specs/2026-09-11-whoop-live-hr-design.md`

**Repos touched (commit policy per repo):**

| Repo | Work | Commits |
|------|------|---------|
| `~/repos/airledger` (engine) | Task 1 | yes, per task |
| `~/repos/airledger-archive` (app) | Tasks 2–8 | yes, per task (user approved at plan handoff, overriding the app CLAUDE.md default) |
| `~/repos/airledger-fitness` (schemas) | Task 9 | yes |

**Environment notes for the implementer:**
- Read `~/repos/airledger/CLAUDE.md` and `~/repos/airledger-archive/CLAUDE.md` first.
- App builds/installs via `dart run tool/brand.dart --config ~/repos/airledger-fitness/ledger.yaml` from `~/repos/airledger-archive`. Device serial `66260DLKX00010` (Pixel 11 Pro), package `com.robertyi.fitness` for the branded build.
- `flutter analyze` (~3s) before every build; info-level lints are ignorable.
- The engine dylib only needs rebuilding for Task 1's tests (`cargo test` handles it); the app consumes schema JSON through the adapter, so app-side work doesn't need a fresh dylib.

---

### Task 1: Engine — `hr_pct` + `hr_max_target` schema keys (Rust)

**Files:**
- Modify: `~/repos/airledger/src/schema/input.rs:75-112`
- Modify: `~/repos/airledger/src/parse/input.rs:26-40,205-276`
- Create: `~/repos/airledger/tests/input_hr.rs`
- Modify: `~/repos/airledger/tests/fixtures/fitness/cardio.input.yml`

- [ ] **Step 1: Write the failing test**

Create `~/repos/airledger/tests/input_hr.rs`:

```rust
//! Round-trip coverage for the HR-driven timer schema keys
//! (`ladders[].hr_pct`, `hr_max_target`) — Whoop live HR integration.

use airledger_engine::parse_input_overlay;
use airledger_engine::schema::input::InputSpec;

const HR_YAML: &str = r#"
target: cardio.view.yml
fields:
  start_time:
    widget: timer
    hr_max_target: max_hr
    ladders:
      - { label: "Zone 4 reached", target: zone4_reached, hr_pct: 80 }
      - { label: "Zone 5 reached", target: zone5_reached }
    stop_target: total_time
"#;

#[test]
fn parses_hr_pct_and_hr_max_target() {
    let overlay = parse_input_overlay(HR_YAML).unwrap();
    let spec = overlay.dimensions["start_time"].input.as_ref().unwrap();
    assert_eq!(spec.hr_max_target.as_deref(), Some("max_hr"));
    let ladders = spec.ladders.as_ref().unwrap();
    assert_eq!(ladders[0].hr_pct, Some(80.0));
    assert_eq!(ladders[1].hr_pct, None);
}

#[test]
fn hr_keys_survive_json_round_trip() {
    // The Dart adapter ships InputSpec as JSON into the engine; a lossy
    // round-trip here would silently drop the feature on device.
    let overlay = parse_input_overlay(HR_YAML).unwrap();
    let spec = overlay.dimensions["start_time"].input.as_ref().unwrap();
    let json = serde_json::to_string(spec).unwrap();
    let back: InputSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, spec);
}

#[test]
fn absent_hr_keys_stay_none_and_are_not_serialized() {
    let overlay = parse_input_overlay(
        "target: cardio.view.yml\n\
         fields:\n\
           start_time:\n\
             widget: timer\n\
             ladders:\n\
               - { label: Z4, target: zone4_reached }\n",
    )
    .unwrap();
    let spec = overlay.dimensions["start_time"].input.as_ref().unwrap();
    assert_eq!(spec.hr_max_target, None);
    assert_eq!(spec.ladders.as_ref().unwrap()[0].hr_pct, None);
    let json = serde_json::to_string(spec).unwrap();
    assert!(!json.contains("hr_pct"), "json: {json}");
    assert!(!json.contains("hr_max_target"), "json: {json}");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd ~/repos/airledger && cargo test --test input_hr`
Expected: COMPILE ERROR — `no field 'hr_pct' on type 'TimerLadder'` / `no field 'hr_max_target'`.

- [ ] **Step 3: Add the schema fields**

In `~/repos/airledger/src/schema/input.rs`, append to `InputSpec` after the `stop_targets` field (line 83):

```rust
    /// For `widget: timer` only. Dim that receives the highest live BPM
    /// observed while the timer ran, written as a number on Stop. Set
    /// by the app's BLE heart-rate feed (Whoop broadcast).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hr_max_target: Option<String>,
```

Replace the `TimerLadder` struct (lines 106-112) with:

```rust
/// One chip in a [`WidgetType::Timer`]'s ladder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerLadder {
    pub label: String,
    /// Dim name on the same view that elapsed time gets stamped into.
    pub target: String,
    /// Auto-stamp threshold as a percent of the user's max heart rate
    /// (ledger meta `user_max_hr`). When set and a live BLE HR source
    /// is connected, the app fires this ladder automatically the first
    /// time live BPM reaches the threshold while the timer runs.
    /// Manual taps keep working either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hr_pct: Option<f64>,
}
```

- [ ] **Step 4: Teach the parser the new keys**

In `~/repos/airledger/src/parse/input.rs`:

Add `"hr_max_target"` to `FORM_SPEC_KEYS` (after `"stop_targets"`, line 39):

```rust
    "stop_targets",
    "hr_max_target",
];
```

In `parse_input_spec` (line 213), add to the `InputSpec { ... }` construction after `stop_targets`:

```rust
        stop_targets,
        hr_max_target: node
            .get(Value::String("hr_max_target".into()))
            .and_then(Value::as_str)
            .map(String::from),
    })
```

Replace `parse_ladders` (lines 262-276) with:

```rust
fn parse_ladders(seq: &[Value]) -> Result<Vec<TimerLadder>, ParseError> {
    seq.iter()
        .map(|v| {
            let m = v.as_mapping().ok_or_else(|| {
                ParseError::Schema(
                    "ladders[]: each entry must be a map with label + target"
                        .into(),
                )
            })?;
            let label = require_string(m, "label")?;
            let target = require_string(m, "target")?;
            let hr_pct = m
                .get(Value::String("hr_pct".into()))
                .and_then(Value::as_f64);
            Ok(TimerLadder { label, target, hr_pct })
        })
        .collect()
}
```

- [ ] **Step 5: Run the new test to verify it passes**

Run: `cargo test --test input_hr`
Expected: `test result: ok. 3 passed`

- [ ] **Step 6: Update the fitness fixture so the real-schema walker covers HR keys**

In `~/repos/airledger/tests/fixtures/fitness/cardio.input.yml`, replace the `start_time:` block's `ladders:` list:

```yaml
    ladders:
      - { label: "Zone 4 reached", target: zone4_reached, hr_pct: 80 }
      - { label: "Zone 5 reached", target: zone5_reached, hr_pct: 90 }
    stop_target: total_time
    hr_max_target: max_hr
```

(Keep the `widget: timer`, `placeholder:` and comment lines as they are.)

- [ ] **Step 7: Run the full engine suite**

Run: `cargo test`
Expected: all tests pass (sheets round-trip skips without env creds). If anything else constructs `TimerLadder` literally, add `hr_pct: None` there — as of writing, `parse_ladders` is the only constructor.

- [ ] **Step 8: Commit (engine repo)**

```bash
cd ~/repos/airledger
git add src/schema/input.rs src/parse/input.rs tests/input_hr.rs tests/fixtures/fitness/cardio.input.yml
git commit -m "feat(schema): hr_pct on timer ladders + hr_max_target

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: App — Dart schema mirrors

**Files:**
- Modify: `~/repos/airledger-archive/lib/models/view_schema.dart:267-345`
- Modify: `~/repos/airledger-archive/lib/services/input_parser.dart:259-364`
- Modify: `~/repos/airledger-archive/lib/services/engine_schema_adapter.dart:92-114,277-327`
- Create: `~/repos/airledger-archive/test/input_hr_schema_test.dart`

- [ ] **Step 1: Write the failing test**

Create `~/repos/airledger-archive/test/input_hr_schema_test.dart`:

```dart
import 'package:flutter_test/flutter_test.dart';
import 'package:airledger/services/input_parser.dart';

const _cardio = '''
target: cardio.view.yml
fields:
  start_time:
    widget: timer
    hr_max_target: max_hr
    ladders:
      - { label: "Zone 4 reached", target: zone4_reached, hr_pct: 80 }
      - { label: "Zone 5 reached", target: zone5_reached }
    stop_target: total_time
''';

void main() {
  test('parses hr_pct and hr_max_target', () {
    final overlay = parseInputOverlay(_cardio);
    final spec = overlay.dimensions['start_time']!.input!;
    expect(spec.hrMaxTarget, 'max_hr');
    expect(spec.ladders![0].hrPct, 80);
    expect(spec.ladders![1].hrPct, isNull);
  });

  test('absent hr keys stay null', () {
    final overlay = parseInputOverlay('''
target: cardio.view.yml
fields:
  start_time:
    widget: timer
    ladders:
      - { label: Z4, target: zone4_reached }
''');
    final spec = overlay.dimensions['start_time']!.input!;
    expect(spec.hrMaxTarget, isNull);
    expect(spec.ladders![0].hrPct, isNull);
  });
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd ~/repos/airledger-archive && flutter test test/input_hr_schema_test.dart`
Expected: COMPILE ERROR — `hrMaxTarget` / `hrPct` not defined.

- [ ] **Step 3: Add the model fields**

In `~/repos/airledger-archive/lib/models/view_schema.dart`:

`InputSpec` — add after the `stopTargets` field (line 302):

```dart
  /// For `widget: timer` fields. Dim that receives the highest live BPM
  /// observed while the timer ran (written as a number on Stop). Fed by
  /// the BLE heart-rate service; null = no HR max capture.
  final String? hrMaxTarget;
```

and add `this.hrMaxTarget,` to the `InputSpec` constructor (after `this.stopTargets,`).

`TimerLadder` (lines 340-345) — replace with:

```dart
class TimerLadder {
  final String label;
  final String target;

  /// Auto-stamp threshold as percent of the user's max HR (ledger meta
  /// `user_max_hr`). When set and a live BLE HR source is connected,
  /// the ladder fires automatically the first time live BPM reaches
  /// the threshold while the timer runs. Null = manual tap only.
  final double? hrPct;

  const TimerLadder({required this.label, required this.target, this.hrPct});
}
```

- [ ] **Step 4: Teach the YAML parser**

In `~/repos/airledger-archive/lib/services/input_parser.dart`:

Add `'hr_max_target',` to the `formKeys` set in `_looksLikeFormSpec` (after `'stop_targets',` line 273).

In `_parseInput` (line 285), add after `stopTargets: _parseStopTargets(node),`:

```dart
    hrMaxTarget: node['hr_max_target'] as String?,
```

In `_parseLadders` (line 348), extend the `TimerLadder` construction:

```dart
    return TimerLadder(
      label: _requireString(entry, 'label'),
      target: _requireString(entry, 'target'),
      hrPct: (entry['hr_pct'] as num?)?.toDouble(),
    );
```

- [ ] **Step 5: Teach the engine JSON adapter (both directions)**

In `~/repos/airledger-archive/lib/services/engine_schema_adapter.dart`:

`_inputSpecToJson` (line 92) — inside the ladders `.map`, and after the `stop_targets` entry:

```dart
      if (s.ladders != null)
        'ladders': s.ladders!
            .map((l) => {
                  'label': l.label,
                  'target': l.target,
                  if (l.hrPct != null) 'hr_pct': l.hrPct,
                })
            .toList(),
      if (s.stopTargets != null)
        'stop_targets': s.stopTargets!
            .map((t) => {
                  'target': t.target,
                  'format': _stopFormatToJson(t.format),
                })
            .toList(),
      if (s.hrMaxTarget != null) 'hr_max_target': s.hrMaxTarget,
```

`_inputSpec` (line 277) — add after `stopTargets: _stopTargets(m['stop_targets']),`:

```dart
    hrMaxTarget: m['hr_max_target'] as String?,
```

`_ladders` (line 318) — extend the construction:

```dart
    return TimerLadder(
      label: m['label'] as String,
      target: m['target'] as String,
      hrPct: (m['hr_pct'] as num?)?.toDouble(),
    );
```

- [ ] **Step 6: Run tests + analyze**

Run: `flutter test test/input_hr_schema_test.dart && flutter analyze`
Expected: `All tests passed!`; analyze clean (info lints ok).

- [ ] **Step 7: Commit (app repo)**

```bash
cd ~/repos/airledger-archive
git add lib/models/view_schema.dart lib/services/input_parser.dart lib/services/engine_schema_adapter.dart test/input_hr_schema_test.dart
git commit -m "feat(schema): mirror hr_pct + hr_max_target in Dart models

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: App — dependencies + Android BLE permissions

**Files:**
- Modify: `~/repos/airledger-archive/pubspec.yaml`
- Modify: `~/repos/airledger-archive/android/app/src/main/AndroidManifest.xml:2`

- [ ] **Step 1: Add packages**

Run: `cd ~/repos/airledger-archive && flutter pub add flutter_blue_plus permission_handler wakelock_plus`
Expected: `Changed N dependencies!` — pins current compatible versions in pubspec.yaml/pubspec.lock. Do NOT run `flutter pub upgrade` (repo convention: pinned versions).

- [ ] **Step 2: Add manifest permissions**

In `AndroidManifest.xml`, directly after the existing `<uses-permission android:name="android.permission.INTERNET"/>` line:

```xml
    <!-- BLE heart rate (Whoop broadcast, service 0x180D). Scan is
         neverForLocation so no location permission is required on
         API 31+. maxSdkVersion entries cover pre-31 devices. -->
    <uses-permission android:name="android.permission.BLUETOOTH"
        android:maxSdkVersion="30"/>
    <uses-permission android:name="android.permission.BLUETOOTH_ADMIN"
        android:maxSdkVersion="30"/>
    <uses-permission android:name="android.permission.BLUETOOTH_SCAN"
        android:usesPermissionFlags="neverForLocation"/>
    <uses-permission android:name="android.permission.BLUETOOTH_CONNECT"/>
```

- [ ] **Step 3: Verify the build still works**

Run: `flutter analyze && flutter build apk --release`
Expected: `Built build/app/outputs/flutter-apk/app-release.apk`.

- [ ] **Step 4: Commit (app repo)**

```bash
git add pubspec.yaml pubspec.lock android/app/src/main/AndroidManifest.xml
git commit -m "feat(ble): flutter_blue_plus + permission_handler + wakelock deps, BLE permissions

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: App — pure HR logic (`decodeHeartRate` + `HrSession`)

**Files:**
- Create: `~/repos/airledger-archive/lib/services/hr_session.dart`
- Create: `~/repos/airledger-archive/test/hr_session_test.dart`

- [ ] **Step 1: Write the failing tests**

Create `~/repos/airledger-archive/test/hr_session_test.dart`:

```dart
import 'package:flutter_test/flutter_test.dart';
import 'package:airledger/models/view_schema.dart';
import 'package:airledger/services/hr_session.dart';

void main() {
  group('decodeHeartRate', () {
    test('uint8 format (flags bit 0 clear)', () {
      expect(decodeHeartRate([0x00, 142]), 142);
    });
    test('uint16 little-endian format (flags bit 0 set)', () {
      expect(decodeHeartRate([0x01, 0x2C, 0x01]), 300);
    });
    test('other flag bits (energy/RR present) do not affect decode', () {
      expect(decodeHeartRate([0x16, 155, 0x10, 0x02]), 155);
    });
    test('malformed payloads return null', () {
      expect(decodeHeartRate([]), isNull);
      expect(decodeHeartRate([0x00]), isNull);
      expect(decodeHeartRate([0x01, 0x2C]), isNull);
    });
  });

  group('HrSession', () {
    const z4 =
        TimerLadder(label: 'Zone 4', target: 'zone4_reached', hrPct: 80);
    const z5 =
        TimerLadder(label: 'Zone 5', target: 'zone5_reached', hrPct: 90);

    test('fires each ladder once, at its threshold', () {
      final s = HrSession(maxHr: 200, ladders: const [z4, z5]);
      expect(s.onSample(150), isEmpty); // 75% of 200 — below zone 4
      expect(s.onSample(160).map((l) => l.target), ['zone4_reached']);
      expect(s.onSample(165), isEmpty); // zone 4 already fired
      expect(s.onSample(185).map((l) => l.target), ['zone5_reached']);
      expect(s.onSample(190), isEmpty);
    });

    test('one spike can fire both zones in a single sample', () {
      final s = HrSession(maxHr: 200, ladders: const [z4, z5]);
      expect(s.onSample(185).map((l) => l.target),
          ['zone4_reached', 'zone5_reached']);
    });

    test('tracks session max across samples', () {
      final s = HrSession(maxHr: 200, ladders: const [z4]);
      s.onSample(120);
      s.onSample(171);
      s.onSample(155);
      expect(s.sessionMax, 171);
    });

    test('null maxHr disables zone firing but still tracks max', () {
      final s = HrSession(maxHr: null, ladders: const [z4, z5]);
      expect(s.onSample(190), isEmpty);
      expect(s.sessionMax, 190);
    });

    test('ladders without hr_pct are ignored', () {
      const manual = TimerLadder(label: 'M', target: 'manual_field');
      final s = HrSession(maxHr: 200, ladders: const [manual, z4]);
      expect(s.onSample(190).map((l) => l.target), ['zone4_reached']);
    });
  });
}
```

- [ ] **Step 2: Run to verify failure**

Run: `flutter test test/hr_session_test.dart`
Expected: COMPILE ERROR — `hr_session.dart` doesn't exist.

- [ ] **Step 3: Implement**

Create `~/repos/airledger-archive/lib/services/hr_session.dart`:

```dart
import '../models/view_schema.dart';

/// Decodes a standard BLE Heart Rate Measurement (characteristic
/// 0x2A37) payload. Flags byte bit 0 selects the BPM width: clear =
/// uint8 at byte 1, set = uint16 little-endian at bytes 1-2. Returns
/// null on malformed data.
int? decodeHeartRate(List<int> data) {
  if (data.isEmpty) return null;
  final wide = data[0] & 0x01 != 0;
  if (wide) {
    if (data.length < 3) return null;
    return data[1] | (data[2] << 8);
  }
  if (data.length < 2) return null;
  return data[1];
}

/// Per-workout HR tracking: session max BPM plus one-shot zone-crossing
/// detection for ladders that declare `hr_pct`. Pure logic — the timer
/// widget feeds it samples and decides what to do with the results
/// (the caller still skips targets whose field already has a value).
class HrSession {
  HrSession({
    required this.maxHr,
    required List<TimerLadder> ladders,
    this.hrMaxTarget,
  }) : ladders =
            ladders.where((l) => l.hrPct != null).toList(growable: false);

  /// User's max heart rate (ledger meta `user_max_hr`). Null disables
  /// zone detection; session max still tracks.
  final int? maxHr;

  /// Only the HR-driven ladders (hr_pct != null).
  final List<TimerLadder> ladders;
  final String? hrMaxTarget;

  int? sessionMax;
  final Set<String> _fired = {};

  /// Feed one BPM sample; returns the ladders whose threshold this
  /// sample crosses for the first time this session.
  List<TimerLadder> onSample(int bpm) {
    if (sessionMax == null || bpm > sessionMax!) sessionMax = bpm;
    final max = maxHr;
    if (max == null || max <= 0) return const [];
    final due = <TimerLadder>[];
    for (final l in ladders) {
      if (_fired.contains(l.target)) continue;
      if (bpm >= max * l.hrPct! / 100) {
        _fired.add(l.target);
        due.add(l);
      }
    }
    return due;
  }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `flutter test test/hr_session_test.dart`
Expected: `All tests passed!` (9 tests).

- [ ] **Step 5: Commit (app repo)**

```bash
git add lib/services/hr_session.dart test/hr_session_test.dart
git commit -m "feat(hr): BLE HR measurement decode + HrSession zone/max tracking

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: App — `HeartRateService` (BLE client)

**Files:**
- Create: `~/repos/airledger-archive/lib/services/heart_rate_service.dart`

No unit test (hardware-bound); correctness is covered by `flutter analyze`, the pure decode tests (Task 4), and Task 10's on-device verification.

- [ ] **Step 1: Implement the service**

Create `~/repos/airledger-archive/lib/services/heart_rate_service.dart`:

```dart
import 'dart:async';

import 'package:airledger_engine/airledger_engine.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_blue_plus/flutter_blue_plus.dart';

import 'hr_session.dart';

/// Connection lifecycle for the live BLE heart-rate feed.
enum HrState { disconnected, connecting, connected, reconnecting }

/// Generic BLE heart-rate client (standard Heart Rate Service 0x180D —
/// Whoop broadcast, Polar/Garmin straps, anything). One instance per
/// app, created at bootstrap next to the IntegrationRegistry.
///
/// Ledger meta keys (shared with the Whoop integration card):
///   - `integration_whoop_device_id` — remembered BLE remote id
///   - `user_max_hr` — user's max HR; drives zone thresholds
class HeartRateService {
  HeartRateService({required this.repo});

  static HeartRateService? instance;

  final EngineLedgerRepository repo;

  static const kDeviceIdKey = 'integration_whoop_device_id';
  static const kMaxHrKey = 'user_max_hr';

  static final serviceHr = Guid('180D');
  static final charHrMeasurement = Guid('2A37');

  final state = ValueNotifier<HrState>(HrState.disconnected);
  final lastBpm = ValueNotifier<int?>(null);

  /// User's max HR, mirrored from ledger meta at [init] and kept in
  /// sync by [setMaxHr]. Widgets read this instead of touching meta.
  final maxHr = ValueNotifier<int?>(null);

  final _bpmController = StreamController<int>.broadcast();
  Stream<int> get bpm => _bpmController.stream;

  BluetoothDevice? _device;
  StreamSubscription<List<int>>? _valueSub;
  StreamSubscription<BluetoothConnectionState>? _connSub;
  bool _wantConnected = false;

  /// Load meta-backed state. Call once at bootstrap.
  Future<void> init() async {
    maxHr.value = int.tryParse(await repo.metaGet(kMaxHrKey) ?? '');
  }

  Future<String?> rememberedDeviceId() async {
    final id = await repo.metaGet(kDeviceIdKey);
    return (id == null || id.isEmpty) ? null : id;
  }

  Future<void> rememberDevice(String remoteId) =>
      repo.metaSet(kDeviceIdKey, remoteId);

  Future<void> setMaxHr(int? value) async {
    await repo.metaSet(kMaxHrKey, value?.toString() ?? '');
    maxHr.value = value;
  }

  /// Connect to the remembered device. Returns false when none is
  /// paired yet (caller sends the user to the Integrations page).
  Future<bool> connectRemembered() async {
    final id = await rememberedDeviceId();
    if (id == null) return false;
    await connectTo(BluetoothDevice.fromId(id));
    return true;
  }

  Future<void> connectTo(BluetoothDevice device) async {
    _wantConnected = true;
    _device = device;
    state.value = HrState.connecting;
    await _connSub?.cancel();
    _connSub = device.connectionState.listen((s) {
      if (s == BluetoothConnectionState.disconnected && _wantConnected) {
        // Unexpected drop (out of range): flag + retry until told to stop.
        state.value = HrState.reconnecting;
        _retryLater();
      }
    });
    await _openAndSubscribe();
  }

  Future<void> _openAndSubscribe() async {
    final device = _device;
    if (device == null) return;
    try {
      await device.connect(timeout: const Duration(seconds: 15));
      final services = await device.discoverServices();
      final hr = services.firstWhere((s) => s.uuid == serviceHr);
      final measurement = hr.characteristics
          .firstWhere((c) => c.uuid == charHrMeasurement);
      await _valueSub?.cancel();
      // Subscribe before enabling notifications so no packet is missed.
      _valueSub = measurement.onValueReceived.listen(_onData);
      await measurement.setNotifyValue(true);
      state.value = HrState.connected;
    } catch (e) {
      debugPrint('HR connect failed: $e');
      if (_wantConnected) {
        state.value = HrState.reconnecting;
        _retryLater();
      }
    }
  }

  Future<void> _retryLater() async {
    await Future<void>.delayed(const Duration(seconds: 3));
    if (!_wantConnected || state.value == HrState.connected) return;
    await _openAndSubscribe();
  }

  void _onData(List<int> data) {
    final v = decodeHeartRate(data);
    if (v == null) return;
    lastBpm.value = v;
    _bpmController.add(v);
  }

  Future<void> disconnect() async {
    _wantConnected = false;
    await _valueSub?.cancel();
    _valueSub = null;
    await _connSub?.cancel();
    _connSub = null;
    try {
      await _device?.disconnect();
    } catch (_) {}
    _device = null;
    lastBpm.value = null;
    state.value = HrState.disconnected;
  }

  /// Disconnect and drop the pairing (Whoop card's Disconnect action).
  /// `user_max_hr` intentionally survives.
  Future<void> forget() async {
    await disconnect();
    await repo.metaSet(kDeviceIdKey, '');
  }
}
```

Implementation note: if the installed flutter_blue_plus version renames any API used above (`onValueReceived`, `connectionState`, `platformName`), follow the package's README migration table — do not downgrade the package.

- [ ] **Step 2: Analyze**

Run: `flutter analyze`
Expected: clean (info lints ok).

- [ ] **Step 3: Commit (app repo)**

```bash
git add lib/services/heart_rate_service.dart
git commit -m "feat(hr): HeartRateService — BLE 0x180D client with reconnect + meta-backed pairing

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: App — Whoop integration card + max-HR dialog + bootstrap

**Files:**
- Create: `~/repos/airledger-archive/lib/ui/widgets/hr_max_dialog.dart`
- Create: `~/repos/airledger-archive/lib/services/integrations/whoop.dart`
- Modify: `~/repos/airledger-archive/lib/ui/home_screen.dart:124-139`

- [ ] **Step 1: Max-HR dialog (shared by card + timer badge)**

Create `~/repos/airledger-archive/lib/ui/widgets/hr_max_dialog.dart`:

```dart
import 'package:flutter/material.dart';

import '../../services/heart_rate_service.dart';

/// Numeric prompt for ledger meta `user_max_hr`. Zone stamps fire at
/// each ladder's hr_pct percent of this value.
Future<void> promptMaxHr(BuildContext context, HeartRateService hr) async {
  final controller =
      TextEditingController(text: hr.maxHr.value?.toString() ?? '');
  final value = await showDialog<int>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text('Max heart rate'),
      content: TextField(
        controller: controller,
        keyboardType: TextInputType.number,
        autofocus: true,
        decoration: const InputDecoration(
          labelText: 'BPM',
          helperText: 'Zone stamps fire at each ladder\'s % of this '
              '(cardio: 80% / 90%).',
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(ctx).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(
          onPressed: () =>
              Navigator.of(ctx).pop(int.tryParse(controller.text)),
          child: const Text('Save'),
        ),
      ],
    ),
  );
  if (value != null && value >= 100 && value <= 230) {
    await hr.setMaxHr(value);
  }
}
```

- [ ] **Step 2: Whoop integration**

Create `~/repos/airledger-archive/lib/services/integrations/whoop.dart`:

```dart
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_blue_plus/flutter_blue_plus.dart';
import 'package:permission_handler/permission_handler.dart';

import '../../ui/widgets/hr_max_dialog.dart';
import '../heart_rate_service.dart';
import 'integration.dart';

/// Whoop live heart rate via the band's BLE Heart Rate Broadcast
/// (standard service 0x180D — any BLE strap works). Live-only: samples
/// flow into the cardio form while the user works out, through the
/// normal form-save path. There is no history to pull, so [pull] is a
/// no-op and ingest/provenance are not involved.
class WhoopIntegration implements Integration {
  WhoopIntegration({required this.hr});

  final HeartRateService hr;

  @override
  String get id => 'whoop';
  @override
  String get displayName => 'Whoop';
  @override
  String get targetDescription => '→ live heart rate';
  @override
  bool get isConfigured => true; // no API credentials involved

  @override
  Future<bool> get isConnected async =>
      (await hr.rememberedDeviceId()) != null;

  @override
  Future<String> get statusLine async {
    final device = await hr.rememberedDeviceId();
    if (device == null) {
      return 'Not paired · turn on HR Broadcast in the Whoop app first';
    }
    final max = hr.maxHr.value;
    return 'Paired · ${max == null ? 'max HR not set' : 'max HR $max'}';
  }

  @override
  Future<void> connect(BuildContext context) async {
    final statuses = await [
      Permission.bluetoothScan,
      Permission.bluetoothConnect,
    ].request();
    if (statuses.values.any((s) => !s.isGranted)) {
      if (context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
          content: Text('Bluetooth permission is needed to find your '
              'Whoop. Enable it in system Settings > Apps.'),
        ));
      }
      return;
    }
    if (!context.mounted) return;
    final device = await showDialog<BluetoothDevice>(
      context: context,
      builder: (_) => const _HrScanDialog(),
    );
    if (device == null) return;
    await hr.rememberDevice(device.remoteId.str);
    if (context.mounted) await promptMaxHr(context, hr);
    unawaited(hr.connectTo(device));
  }

  @override
  Future<void> disconnect() => hr.forget();

  /// Live-only source: nothing to pull. Interface contract: never throw.
  @override
  Future<void> pull({bool force = false, bool fullReconcile = false}) async {}
}

/// Scans for devices advertising Heart Rate (0x180D) and lets the user
/// pick one. The Whoop only advertises while HR Broadcast is enabled.
class _HrScanDialog extends StatefulWidget {
  const _HrScanDialog();

  @override
  State<_HrScanDialog> createState() => _HrScanDialogState();
}

class _HrScanDialogState extends State<_HrScanDialog> {
  StreamSubscription<List<ScanResult>>? _sub;
  List<ScanResult> _results = const [];
  bool _scanning = true;

  @override
  void initState() {
    super.initState();
    _sub = FlutterBluePlus.scanResults.listen((r) {
      if (mounted) setState(() => _results = r);
    });
    FlutterBluePlus.startScan(
      withServices: [HeartRateService.serviceHr],
      timeout: const Duration(seconds: 15),
    ).then((_) {
      if (mounted) setState(() => _scanning = false);
    });
  }

  @override
  void dispose() {
    _sub?.cancel();
    FlutterBluePlus.stopScan();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Find heart rate source'),
      content: SizedBox(
        width: double.maxFinite,
        child: _results.isEmpty
            ? Text(_scanning
                ? 'Scanning… enable HR Broadcast in the Whoop app '
                    '(Device Settings) and keep the band nearby.'
                : 'Nothing found. Is HR Broadcast on?')
            : ListView(
                shrinkWrap: true,
                children: [
                  for (final r in _results)
                    ListTile(
                      leading: const Icon(Icons.monitor_heart_outlined),
                      title: Text(r.device.platformName.isEmpty
                          ? r.device.remoteId.str
                          : r.device.platformName),
                      subtitle: Text('${r.rssi} dBm'),
                      onTap: () => Navigator.of(context).pop(r.device),
                    ),
                ],
              ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
      ],
    );
  }
}
```

Known quirk (accepted): the generic integration card shows "Sync now" and "Full reconcile" for connected integrations; for Whoop both are harmless no-ops (`pull()` does nothing, then the normal ledger sync runs). Disconnect works via `forget()`.

- [ ] **Step 3: Bootstrap — service singleton + registry entry**

In `~/repos/airledger-archive/lib/ui/home_screen.dart`, add imports (with the other service imports at the top):

```dart
import '../services/heart_rate_service.dart';
import '../services/integrations/whoop.dart';
```

Then replace the `IntegrationRegistry.init` block (lines 131-139) with:

```dart
      final hrService = HeartRateService(repo: repo.repo);
      HeartRateService.instance = hrService;
      await hrService.init();
      IntegrationRegistry.init(integrations: [
        if (weightView != null)
          WithingsIntegration(
            config: assetConfig.withings,
            repo: repo.repo,
            weightViewJson: viewSchemaToEngineJson(weightView),
          ),
        WhoopIntegration(hr: hrService),
        ComingSoonIntegration('Macrofactor', '→ meals (via Health Connect)'),
      ]);
```

- [ ] **Step 4: Analyze + commit (app repo)**

Run: `flutter analyze`
Expected: clean.

```bash
git add lib/ui/widgets/hr_max_dialog.dart lib/services/integrations/whoop.dart lib/ui/home_screen.dart
git commit -m "feat(integrations): Whoop card — BLE pairing, max-HR setting, live-only pattern

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: App — form screen links `hr_max_target` like a timer target

**Files:**
- Modify: `~/repos/airledger-archive/lib/ui/form_screen.dart:81-96,259-267`

- [ ] **Step 1: Include hrMaxTarget in `_timerLinkedFields`**

At form_screen.dart:84-96, add the hrMaxTarget lines so the block reads:

```dart
  late final Set<String> _timerLinkedFields = (() {
    final out = <String>{};
    for (final d in widget.view.editableDimensions) {
      if (d.input?.widget != WidgetType.timer) continue;
      for (final l in d.input?.ladders ?? const <TimerLadder>[]) {
        out.add(l.target);
      }
      for (final s in d.input?.stopTargets ?? const <TimerStopTarget>[]) {
        out.add(s.target);
      }
      final hrMax = d.input?.hrMaxTarget;
      if (hrMax != null) out.add(hrMax);
    }
    return out;
  })();
```

- [ ] **Step 2: Include hrMaxTarget in the `timerLinkedValues` snapshot**

At form_screen.dart:259-267, extend the map:

```dart
        timerLinkedValues: dim.input?.widget == WidgetType.timer
            ? {
                for (final l in dim.input?.ladders ?? const <TimerLadder>[])
                  l.target: _shared[l.target],
                for (final s
                    in dim.input?.stopTargets ?? const <TimerStopTarget>[])
                  s.target: _shared[s.target],
                if (dim.input?.hrMaxTarget != null)
                  dim.input!.hrMaxTarget!: _shared[dim.input!.hrMaxTarget!],
              }
            : null,
```

- [ ] **Step 3: Analyze + commit (app repo)**

Run: `flutter analyze`
Expected: clean.

```bash
git add lib/ui/form_screen.dart
git commit -m "feat(form): hr_max_target participates in timer field linkage

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: App — timer widget: HR badge, auto-stamp, max-HR write, wakelock

**Files:**
- Modify: `~/repos/airledger-archive/lib/ui/widgets/field_widgets.dart` (imports; `_TimerFieldWidgetState`; new `_HrBadge`; `_FullscreenTimerDialog.build`)

- [ ] **Step 1: Imports**

At the top of field_widgets.dart, next to the existing imports:

```dart
import 'package:wakelock_plus/wakelock_plus.dart';

import '../../services/heart_rate_service.dart';
import '../../services/hr_session.dart';
import 'hr_max_dialog.dart';
```

(`dart:async` is already imported for `Timer`.)

- [ ] **Step 2: State fields + lifecycle in `_TimerFieldWidgetState`**

Add fields after `Timer? _ticker;` (line 661):

```dart
  StreamSubscription<int>? _bpmSub;
  HrSession? _hrSession;

  /// True when this timer's schema declares any HR behavior — a ladder
  /// with hr_pct or an hr_max_target. Gates all HR UI and subscriptions.
  bool get _hrConfigured =>
      widget.dim.input?.hrMaxTarget != null ||
      (widget.dim.input?.ladders?.any((l) => l.hrPct != null) ?? false);
```

Extend `initState` (line 665) to:

```dart
  @override
  void initState() {
    super.initState();
    _controller = TextEditingController(text: widget.value?.toString() ?? '');
    final hr = HeartRateService.instance;
    if (_hrConfigured && hr != null) {
      _bpmSub = hr.bpm.listen(_onBpm);
      hr.state.addListener(_onHrChanged);
      hr.maxHr.addListener(_onHrChanged);
    }
  }

  void _onHrChanged() {
    if (mounted) setState(() {});
  }
```

Extend `dispose` (line 671) to:

```dart
  @override
  void dispose() {
    _ticker?.cancel();
    _bpmSub?.cancel();
    final hr = HeartRateService.instance;
    hr?.state.removeListener(_onHrChanged);
    hr?.maxHr.removeListener(_onHrChanged);
    WakelockPlus.disable();
    _controller.dispose();
    super.dispose();
  }
```

- [ ] **Step 3: The BPM handler**

Add after `_onHrChanged`:

```dart
  /// Live sample: track session max; auto-stamp any hr_pct ladder whose
  /// threshold this sample crosses — same guard as a manual tap (only
  /// blank targets), same write path (onLadderTap).
  void _onBpm(int bpm) {
    if (!mounted) return;
    final running = _ticker != null && _startedAt != null && !_paused;
    final session = _hrSession;
    if (!running || session == null) {
      setState(() {}); // badge refresh only
      return;
    }
    final due = session.onSample(bpm);
    for (final ladder in due) {
      if (_isNonEmpty(widget.linkedValues[ladder.target])) continue;
      final elapsed = _liveElapsed();
      if (elapsed == null) continue;
      final formatted = _formatElapsed(elapsed);
      widget.onLadderTap?.call(ladder.target, formatted);
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text('${ladder.label}: $formatted · auto (HR $bpm)'),
          duration: const Duration(milliseconds: 1200),
        ),
      );
    }
    setState(() {});
  }
```

- [ ] **Step 4: Hook `_start` and `_stop`**

In `_start()` (line 701), inside the final `setState`, after `_stopped = false;`:

```dart
      _hrSession = HrSession(
        maxHr: HeartRateService.instance?.maxHr.value,
        ladders: widget.dim.input?.ladders ?? const [],
        hrMaxTarget: widget.dim.input?.hrMaxTarget,
      );
```

and directly after that `setState(...)` call (still inside `_start`):

```dart
    unawaited(WakelockPlus.enable());
```

In `_stop()` (line 776), before the existing `setState`, after the stop-targets loop + snackbar:

```dart
    final hrTarget = widget.dim.input?.hrMaxTarget;
    final sessionMax = _hrSession?.sessionMax;
    if (hrTarget != null && sessionMax != null) {
      // Highest BPM seen this session → e.g. max_hr. A later manual
      // edit wins; it's a normal form field.
      widget.onLadderTap?.call(hrTarget, sessionMax);
    }
    unawaited(WakelockPlus.disable());
```

- [ ] **Step 5: The badge widget**

Add a new top-level widget at the end of field_widgets.dart:

```dart
/// Live BPM chip for HR-configured timers. Shows connection state, the
/// current BPM colored by zone, a connect affordance when disconnected,
/// and a "set max HR" prompt when unset. Zone colors are display-only
/// approximations (80%/90% of max HR); stamping thresholds come from
/// the schema's hr_pct values.
class _HrBadge extends StatelessWidget {
  const _HrBadge();

  @override
  Widget build(BuildContext context) {
    final hr = HeartRateService.instance;
    if (hr == null) return const SizedBox.shrink();
    final scheme = Theme.of(context).colorScheme;
    return ValueListenableBuilder<HrState>(
      valueListenable: hr.state,
      builder: (context, state, _) {
        switch (state) {
          case HrState.disconnected:
            return ActionChip(
              avatar: Icon(Icons.favorite_border,
                  size: 16, color: scheme.onSurfaceVariant),
              label: const Text('Connect HR'),
              onPressed: () async {
                final ok = await hr.connectRemembered();
                if (!ok && context.mounted) {
                  ScaffoldMessenger.of(context).showSnackBar(const SnackBar(
                    content: Text(
                        'Pair your Whoop on the Integrations page first.'),
                  ));
                }
              },
            );
          case HrState.connecting:
            return const Chip(
              avatar: SizedBox(
                width: 12,
                height: 12,
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
              label: Text('Connecting…'),
            );
          case HrState.reconnecting:
            return Chip(
              avatar:
                  Icon(Icons.sync_problem, size: 16, color: scheme.error),
              label: const Text('Reconnecting…'),
            );
          case HrState.connected:
            return ValueListenableBuilder<int?>(
              valueListenable: hr.lastBpm,
              builder: (context, bpm, _) {
                final max = hr.maxHr.value;
                if (max == null) {
                  return ActionChip(
                    avatar:
                        Icon(Icons.favorite, size: 16, color: scheme.primary),
                    label: Text(
                        bpm == null ? '— bpm' : '$bpm bpm · set max HR'),
                    onPressed: () => promptMaxHr(context, hr),
                  );
                }
                var color = scheme.onSurfaceVariant;
                if (bpm != null && bpm >= max * 0.9) {
                  color = Colors.red;
                } else if (bpm != null && bpm >= max * 0.8) {
                  color = Colors.orange;
                }
                return Chip(
                  avatar: Icon(Icons.favorite, size: 16, color: color),
                  label: Text(
                    bpm == null ? '— bpm' : '$bpm bpm',
                    style:
                        TextStyle(color: color, fontWeight: FontWeight.w700),
                  ),
                );
              },
            );
        }
      },
    );
  }
}
```

`Colors` comes from the existing material import.

- [ ] **Step 6: Render the badge in the embedded widget**

In `_TimerFieldWidgetState.build`, directly after the `TextField(...)` child (after line 940) and before the `if (elapsedNow != null)` block, add:

```dart
          if (_hrConfigured)
            const Padding(
              padding: EdgeInsets.only(top: 8),
              child: Align(
                alignment: Alignment.centerLeft,
                child: _HrBadge(),
              ),
            ),
```

(Rendered even while idle, so the user can connect before tapping Start.)

- [ ] **Step 7: Render the badge in the fullscreen dialog**

In `_FullscreenTimerDialogState.build`, in the top `Row` (field_widgets.dart:1182-1228), between `const Spacer()` and the state-label `Container(...)`:

```dart
                  const Spacer(),
                  if (widget.host._hrConfigured) ...[
                    const _HrBadge(),
                    const SizedBox(width: 8),
                  ],
                  Container(
```

- [ ] **Step 8: Analyze + tests + commit (app repo)**

Run: `flutter analyze && flutter test`
Expected: analyze clean; all tests pass (schema, hr_session, and pre-existing suites).

```bash
git add lib/ui/widgets/field_widgets.dart
git commit -m "feat(timer): live HR badge, zone auto-stamp, hr_max_target write, wakelock

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 9: Live schema — enable HR on the real cardio tracker

**Files:**
- Modify: `~/repos/airledger-fitness/views/cardio.input.yml` (the `start_time:` block)

- [ ] **Step 1: Edit the live schema**

Replace the `ladders:`/`stop_target:` lines under `start_time:` with:

```yaml
    ladders:
      - { label: "Zone 4 reached", target: zone4_reached, hr_pct: 80 }
      - { label: "Zone 5 reached", target: zone5_reached, hr_pct: 90 }
    stop_target: total_time
    hr_max_target: max_hr
```

(Keep `widget: timer`, `placeholder:`, and the explanatory comment as they are.)

- [ ] **Step 2: Commit (fitness repo)**

```bash
cd ~/repos/airledger-fitness
git add views/cardio.input.yml
git commit -m "feat(cardio): HR-driven zone stamps + max_hr capture on the 4x4 timer

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 10: Build, install, verify on device

**Files:** none (verification only). Device: Pixel 11 Pro, serial `66260DLKX00010` (already set in `~/repos/airledger-fitness/ledger.yaml`).

- [ ] **Step 1: Build + install the branded app**

```bash
cd ~/repos/airledger-archive
dart run tool/brand.dart --config ~/repos/airledger-fitness/ledger.yaml
```

Expected: syncs schemas (cardio.input.yml with hr keys), builds, installs on `66260DLKX00010`, launches.

- [ ] **Step 2: On-device manual checklist (user does the workout parts)**

1. Integrations page shows a **Whoop → live heart rate** card, "Not paired…" status.
2. Enable HR Broadcast in the Whoop app → tap Connect → grant Bluetooth permissions → Whoop appears in the scan dialog → pick it → max HR prompt → save.
3. Card status becomes `Paired · max HR <n>`.
4. Open a new cardio entry: timer block shows the HR chip; tap **Connect HR** → chip turns into live BPM within a few seconds.
5. Tap Start: BPM colors by zone as HR climbs; when HR first crosses 80% of max, `zone4_reached` fills with the elapsed time (snackbar `Zone 4 reached: m:ss · auto (HR nnn)`); same for zone 5 at 90%; screen does not sleep.
6. Tap Stop: `total_time` fills as before AND `max_hr` fills with the session's highest BPM.
7. Save the row; timeline subtitle shows `<total> / <max>bpm`; row syncs to the cardio sheet on next sync.
8. Reconnect resilience: walk out of BLE range mid-run → chip shows "Reconnecting…", manual ladder chips still work; walk back → BPM resumes, stamping continues.
9. Fullscreen timer shows the same BPM chip next to the state label.

- [ ] **Step 3: Log check if anything misbehaves**

```bash
adb -s 66260DLKX00010 logcat -d --pid=$(adb -s 66260DLKX00010 shell pidof com.robertyi.fitness) | grep -i "flutter\|bluetooth" | tail -50
```

`HR connect failed: …` debugPrints from HeartRateService land under the `flutter` tag. Remove any temporary debugPrints before final commit.

---

## Self-review notes

- Spec coverage: schema keys (T1/T2), max HR meta + card editing (T5/T6), BLE service incl. decode/reconnect/remembered device (T4/T5), integration card with no-op pull (T6), timer badge/auto-stamp/max-write/wakelock (T7/T8), Android permissions (T3), live schema (T9), Rust + Dart tests (T1/T2/T4), manual device pass (T10).
- Deliberate scope cuts, consistent with spec: no iOS work; no full HR time-series persistence; zone colors in the badge are display-only 80/90 approximations while stamping uses schema hr_pct.
- Type consistency: `hrPct` is `double?` end-to-end in Dart, `Option<f64>` in Rust; `hr_max_target` writes an `int` BPM via `onLadderTap(target, sessionMax)` into a `number` dim — matches `max_hr: { widget: number }`.
