# Daily Notes — Design

**Date:** 2026-09-11
**Status:** Approved
**Parent effort:** AI daily coach, sub-project 1 of 4 (notes → goals/routine
→ derived metrics → nightly coach).

## Goal

One free-form journal entry per day — how training felt, sleep, life
context. Feeds the user's daily review today and the nightly AI coach
later. Deliberately minimal: no structured scalars, no dedicated UI.

## Design

Pure schema addition in `~/repos/airledger-fitness/views/` — no app or
engine code.

**`daily_notes.view.yml`** (weight-view pattern):
- `datasource: gsheets`, `table: daily_notes` (tab auto-created in the
  main workbook by additive `ensureSheet` on first use)
- entities: primary `daily_note`, key `id`
- dimensions: `id` (string, UUID), `date` (date), `day_of_week`
  (string), `note` (string)

**`daily_notes.input.yml`**:
- `icon: notebook-pen`, `date_field: date`
- `list_display`: `title: note`, `subtitle: ${day_of_week}`
- fields: `id` `editable: false`; `date` widget date, `default: today`,
  required; `day_of_week` derived from date (`weekday_long`); `note`
  widget longtext, required, placeholder guiding content.

## Behavior

- Home-screen tile appears automatically (schema-driven).
- One row per day **by convention**: edit today's row from the timeline
  to append/revise. No uniqueness enforcement — a duplicate day is
  harmless, and the coach will read all notes per date.
- Syncs to Sheets via the normal local-first pipeline; that synced copy
  is what the nightly coach will read.

## Deployment

Commit + push `airledger-fitness`. SchemaSync pulls views from GitHub at
runtime, so the view should appear without an app rebuild; the bundled-
assets rebuild (`brand.dart`) is the fallback if it doesn't.

## Testing

- `dart run tool/check_schema.dart` (app repo) against the new files.
- On device: tile appears, a note saves, `daily_notes` tab materializes
  in the workbook, row syncs.

## Out of scope (later sub-projects)

Today-dashboard note card; structured energy/soreness scalars; goals &
routine codification; derived metrics; the nightly coach run itself.
