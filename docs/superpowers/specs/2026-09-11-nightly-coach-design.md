# Nightly AI coach — Design (sub-projects 3 + 4)

**Date:** 2026-09-11
**Status:** Approved in decomposition; details decided autonomously per
the user's "continue without any verification from me until the tasks
are all completed."

## Goal

Once a day (~midnight, on the Mac, using the user's **Claude Max plan**
— no API credits), assess the ledger + goals/routine and (a) write
tomorrow's workout into the app as **draft entries**, (b) write a
readable summary the user reads in the app.

## Key facts driving the design (verified in code)

- PlanStore ("planned" entries) is device-local shared_preferences —
  a Mac job cannot write it.
- Externally appended sheet rows ARE picked up by sync
  (`Action::TakeRemote`) and render on that date's timeline — but as
  normal logged rows, not "planned."
- New views ship via SchemaSync from GitHub (5-min poll) without an
  app rebuild.
- Standalone Dart tools in `airledger-archive/tool/` already auth to
  Sheets via `~/.config/airledger/service-account.json`
  (migrate_*.dart pattern).

## Components

### 1. Derived metrics (sub-project 3) — `coach/metrics.md`

Formula definitions the coach applies to raw dump data at run time
(Epley e1RM + 4-week maintenance slope, weekly tonnage, ACWR fatigue
with daily-notes override, 4×4 quality/progress, 7-day trend weight,
weekly adherence vs routine). Nothing is precomputed or stored; the
doc is prompt context.

### 2. `coach_log` view (fitness repo)

`date` (day being planned), `generated_at`, `summary` (longtext),
`drafted` (one line per draft group). Ships via SchemaSync; the app
gets a "coach_log" tile where the user reads the morning summary.

### 3. Draft treatment in the app (timeline)

On plannable views, a synced row whose `plannable.log_field` is blank
renders as a **Draft**: grouped under a "Draft" header on its date,
visually badged, with a "Log now" action that STAMPS the log_field on
the existing row (repository.update) instead of creating a new row.
Coexists with PlanStore planned items (which keep their current
behavior). This is what makes coach-written rows first-class plans.

### 4. Mac-side nightly pipeline

- `tool/coach_dump.dart` (app repo): reads the last 28 days of
  strength, cardio, weight, daily_notes (+ recent coach_log) from the
  main workbook via the service account; emits a markdown context dump
  (per-view tables, dates, all columns).
- `coach/PROMPT.md` (fitness repo): the run instruction — role, read
  goals/routine/metrics + dump, decide tomorrow per routine rules and
  daily notes, output STRICT JSON:
  `{"summary": str, "drafted": str, "plan": [{"view": str, "rows": [{dim: value}]}]}`
  with blank log_field on plan rows, date = tomorrow.
- `tool/coach_apply.dart` (app repo): validates the JSON against the
  fitness-repo schemas (view + dimension names), assigns UUIDs and
  day_of_week, appends plan rows + one coach_log row via the Sheets
  API. Refuses unknown views/fields; never edits existing rows.
- `tool/coach_nightly.sh` (app repo): dump → assemble prompt (PROMPT +
  goals + routine + metrics + dump) → `claude -p` (headless Claude
  Code on the user's Max login) → apply → log to
  `~/.config/airledger/coach/logs/<date>.log`. Any step failing aborts
  the run — no partial writes before apply validation.
- launchd agent `com.robertyi.airledger-coach` at 00:05 daily.
  Disable with `launchctl unload ~/Library/LaunchAgents/com.robertyi.airledger-coach.plist`.

## Error handling

- Apply validates before any write; malformed model output → logged,
  nothing written.
- Sheets tab for coach_log is created by the app's ensureSheet on
  first tile use; apply also creates-if-missing with headers.
- A rerun on the same night: apply skips if a coach_log row for the
  target date already exists (idempotence guard).

## Testing

- Tool-level: dump runs against the real workbook (read-only);
  apply has a `--dry-run` flag printing what it would write.
- One live end-to-end run at build time — the draft rows + summary it
  writes for tomorrow ARE the acceptance test the user wakes up to.
