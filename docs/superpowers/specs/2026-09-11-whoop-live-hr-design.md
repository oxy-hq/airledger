# Whoop Live Heart Rate Integration — Design

**Date:** 2026-09-11
**Status:** Approved

## Goal

Show live heart rate from a Whoop band (BLE Heart Rate Broadcast) inside the
cardio entry form during a workout, and auto-fill the cardio view's
HR-derived fields — `zone4_reached`, `zone5_reached` (elapsed at zone
crossing) and `max_hr` (session max BPM) — which are today stamped by hand.

Whoop's developer API does not expose continuous HR; the live path is the
standard BLE Heart Rate Service (0x180D) the band advertises when
"HR Broadcast" is enabled in the Whoop app. The integration is therefore
generic BLE HR — any strap that speaks the profile works.

Out of scope: persisting the full HR time series; iOS (Android/Pixel is the
target); background/ambient HR capture outside the form.

## Architecture

Data flow:

```
Whoop band (BLE HR broadcast, 0x180D/0x2A37)
  → HeartRateService (Flutter app, flutter_blue_plus)
  → timer widget on the cardio form (live BPM badge, zone auto-stamp)
  → existing onLadderTap path fills form fields
  → normal form save persists the row (no ingest pipeline involved)
```

The user is present and saving the form, so HR values flow through the
form-edit path, not `ledger_ingest`/provenance — those stay reserved for
background pull integrations like Withings.

## Components

### 1. Input-schema extension (Rust: `src/schema/input.rs`, `src/parse/input.rs`)

Both additions optional; absent keys mean unchanged behavior.

- `TimerLadder.hr_pct: Option<f64>` — auto-stamp this ladder chip when live
  HR first reaches this percent of the user's configured max HR.
- `InputSpec.hr_max_target: Option<String>` — dim name (on `widget: timer`
  fields) that receives the highest BPM observed while the timer runs,
  written on Stop as a number.

Wire format follows the existing serde pattern
(`skip_serializing_if = "Option::is_none"`), so the engine round-trip
preserves the keys and older schemas parse unchanged.

`cardio.input.yml` (fixture and live sheet copy) becomes:

```yaml
start_time:
  widget: timer
  hr_max_target: max_hr
  ladders:
    - { label: "Zone 4 reached", target: zone4_reached, hr_pct: 80 }
    - { label: "Zone 5 reached", target: zone5_reached, hr_pct: 90 }
  stop_target: total_time
```

Dart mirrors in the app — `lib/models/view_schema.dart`,
`lib/services/input_parser.dart`, `lib/services/engine_schema_adapter.dart` —
gain the same two fields.

### 2. Max HR setting (ledger meta)

- Key: `user_max_hr` (integer BPM), read/written via the existing meta
  get/set FFI.
- Edited on the Whoop integration card.
- Zone thresholds = `hr_pct / 100 × user_max_hr` (Whoop's zone definitions:
  Zone 4 = 80%, Zone 5 = 90%).
- If unset, auto-stamping and zone coloring are disabled; the timer widget's
  HR badge shows a "set max HR" prompt linking to the integrations screen.

### 3. `HeartRateService` (app: `lib/services/heart_rate_service.dart`)

Singleton BLE client on `flutter_blue_plus`:

- **Scan:** filter on advertised service 0x180D.
- **Connect + subscribe:** Heart Rate Measurement characteristic 0x2A37,
  notifications on.
- **Decode:** flags byte bit 0 → BPM is uint8 (bit clear) or uint16
  little-endian (bit set).
- **API:** `Stream<int> bpm`, `ValueNotifier<HrConnectionState> state`
  (disconnected / scanning / connecting / connected / reconnecting),
  `connect()`, `connectRemembered()`, `disconnect()`, `forget()`.
- **Remembered device:** BLE remote ID stored in ledger meta
  `integration_whoop_device_id` for one-tap reconnect.
- **Reconnect:** on unexpected drop, retry with backoff while any listener
  is active; surface state as `reconnecting`.

### 4. `WhoopIntegration` (app: `lib/services/integrations/whoop.dart`)

Implements the existing `Integration` interface so it appears as a card on
the integrations screen:

- `connect(context)` — permission request, then scan-and-pick dialog
  (device name + signal), saves the chosen device ID.
- `pull()` — no-op (live-only source; must never throw, per interface).
- `isConfigured` — always true (no API credentials involved).
- `statusLine` — paired device name + `max HR <n>` when set, e.g.
  `Paired: WHOOP 4A0… · max HR 185`.
- `disconnect()` — forget device ID; `user_max_hr` is left in place.
- Card UI additionally exposes the `user_max_hr` numeric setting.

### 5. Timer widget (app: `lib/ui/widgets/field_widgets.dart`)

When the timer field's InputSpec has HR config (any `hr_pct` ladder or an
`hr_max_target`):

- **Badge:** live BPM next to the elapsed badge, colored by current zone
  (below Z4 / Z4 / Z5). Disconnected → a small connect button (uses the
  remembered device; falls back to the scan dialog). Reconnecting → shown
  in the badge.
- **Auto-stamp:** while the timer is running, when BPM ≥ threshold for a
  ladder whose target field is still blank, invoke the existing
  `onLadderTap(target, elapsed)` — byte-for-byte the same effect as a
  manual chip tap. Already-filled targets are never overwritten. Manual
  taps keep working as a fallback throughout.
- **Max HR:** track the running max BPM during the run; on Stop, write it
  to `hr_max_target` via the same callback path. Manual edits after Stop
  win (it's a normal form field).
- **Wakelock:** screen stays awake while the timer runs (`wakelock_plus`),
  released on Stop/dispose.
- Without HR schema config or without a paired device, the widget renders
  exactly as today.

### 6. Permissions (Android)

- Manifest: `BLUETOOTH_SCAN` with `android:usesPermissionFlags="neverForLocation"`,
  `BLUETOOTH_CONNECT` (both API 31+; legacy `BLUETOOTH`/`BLUETOOTH_ADMIN`
  entries with `maxSdkVersion="30"`).
- Runtime request via `permission_handler` before first scan, triggered
  from the connect flow with a friendly rationale on denial.

New app dependencies: `flutter_blue_plus`, `permission_handler`,
`wakelock_plus`.

## Error handling

- **BLE drop mid-workout:** badge → reconnecting; auto-stamp pauses (no
  data, no stamps); manual chips unaffected. On reconnect, streaming and
  stamping resume; max-HR tracking keeps its prior max.
- **No max HR set:** live BPM still displays; stamping and zone colors off.
- **Permission denied:** connect flow shows rationale + link to app
  settings; the rest of the form is unaffected.
- **Whoop broadcast off:** scan finds nothing; dialog explains how to
  enable HR Broadcast in the Whoop app.

## Testing

- **Rust:** parser round-trip tests for `hr_pct` and `hr_max_target`
  (present, absent, legacy schemas); fitness fixture updated.
- **Dart unit:** HR measurement decoding (uint8/uint16 flag variants);
  zone-stamp logic against a fake BPM stream behind the service interface
  (crossing stamps once, blank-only guard, max tracking across a
  disconnect).
- **Manual:** on the Pixel with the Whoop broadcasting — pair from the
  integrations card, run a cardio session, verify live badge, auto-stamps,
  max_hr on Stop, and survival of a walk-out-of-range reconnect.
