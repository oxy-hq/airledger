# Coach Intent Layers — Implementation Plan (phases B0, A, B)

> **For agentic workers:** execute task-by-task with review between
> tasks. The SPEC is the contract: read
> `docs/superpowers/specs/2026-09-20-coach-intent-layers-spec.md`
> (this repo) before any task. This plan deliberately specifies
> contracts + acceptance rather than full code — the spec's formulas
> are exact and duplicating them here would only invite drift.

**Goal:** Intent (versioned program/phase/strategy YAML) + Outcome
(effort-graded weekly metrics, flags, program_status) layers for the
coach, gated on the §6 backtest reproducing history.

**Repos:** metrics + tools in `~/repos/ledger`; YAML seeds in
`~/repos/airledger-fitness/coach/`; MCP in `~/repos/ledger-mcp`.
Baselines: ledger `flutter analyze` 32 infos / `flutter test` exactly 7
known failures; ledger-mcp `npm test` green. Conventional commits with
the Claude trailer, per task.

**Phase C and D are explicitly OUT of scope for this plan.**

---

### Task B0.1 — metrics core (pure Dart, TDD)

Create `~/repos/ledger/lib/services/program_metrics.dart` +
`test/program_metrics_test.dart`. Pure functions, zero Flutter/IO
imports:

- Types: `GradedSet` (date, lift, weight, reps, e1rm, reference,
  effort, pctMax, tier, working, nearMax, longFailureSet, rpe?),
  `WeeklyMetrics` (all §2.5 weekly fields, ISO week Mon–Sun keyed by
  its Monday date), `FlagHit` (id, firedOn, evidence map, action).
- `gradeSets(List<StrengthRow>)` → `List<GradedSet>` implementing §2.5
  EXACTLY: main-lift name table verbatim (5 names → 4 lifts), Epley
  with reps capped at 12, reference = max set_e1rm over same lift with
  reps ≤ 8 in the 42 days ending on and including the set's date,
  carry-forward when window empty (undefined reference until first
  qualifying set → those sets skip grading), tier/working/near_max/
  long_failure thresholds verbatim.
- `weeklyRollup(...)` taking graded sets + weight rows + cardio rows +
  climbing rows (+ program resolver for week_type, nullable for
  backtest of pre-program history) → `List<WeeklyMetrics>`; bw_7d_avg
  as-of-Sunday semantics per spec (trailing 7-day mean of logged
  weights; define: mean of weight_lbs values with date in
  [Sunday-6, Sunday]; missing days just shrink the sample; null when
  no weigh-ins in window).
- `evaluateFlags(List<WeeklyMetrics>, {phase, program})` → per-week
  `List<FlagHit>` for all §2.6 rules computable from history
  (WEIGHT_DRIFT/BLOCK_END/PAIN_NOTE/TOP_SET_HEAVY need
  program-line/test-week/notes/rpe inputs — implement them
  data-permitting; where an input is absent for a historical week the
  rule simply doesn't fire).
TDD: synthetic fixtures proving each formula edge (reps cap, 42-day
boundary inclusive, carry-forward, ISO week boundary, two-week
consecutive conditions, TWO_SIGNALS composition).

### Task B0.2 — backtest tool (§6 gate)

`~/repos/ledger/tool/coach_backtest.dart` (pattern: coach_dump.dart —
service account from ~/.config/airledger, full-tab reads, schema from
~/repos/airledger-fitness). Reads FULL strength + weight + cardio
(+ daily_notes for future PAIN_NOTE) history from the workbook, runs
B0.1, prints: per-§6-check PASS/FAIL with the actual numbers, the
weekly table for the windows §6 names, and the RPE-band sanity table.
Iterate on interpretation ambiguities (e.g. header names, date
formats, duplicate exercise spellings) until checks 1–5 pass — but
NEVER bend the spec formulas; if a check cannot pass without changing
a formula, STOP and report the discrepancy with evidence. §6 verbatim:
this gate blocks everything downstream.

### Task A.1 — intent YAML seeds (user gate on strategy text)

`~/repos/airledger-fitness/coach/program.yaml`, `phase.yaml`,
`strategy.yaml`. Shape: top-level `versions: [ {version, effective_from,
reason, ...payload} ]`, current = last non-pending entry. program v1
payload = §5.1–5.5 verbatim (blocks, week_types, weekly_template,
loads, targets, rules [flag ids], nutrition, hard_cap_lb, band_lb).
phase v1 = §5.6. strategy v1 text = §5.7 ONLY after the user has
edited/approved it in chat. routine.md gets a deprecation banner
(retired in favour of program.weekly_template; readable one release).
Push (SchemaSync trap #2 does not apply to coach/, but the nightly and
worker read from GitHub — push is required for them).

### Task A.2 — program resolver ×2 + shared fixtures

- Dart: `~/repos/ledger/lib/services/program_current.dart` —
  `ProgramSlice programCurrent(programYaml, DateTime date)` returning
  §3's program object (block, week_in_block from block start Monday,
  week_type via week_types rules + block length, today_template,
  targets_in_force, rules_in_force).
- TS: same logic in `~/repos/ledger-mcp/src/program.ts`.
- Shared fixtures: `~/repos/airledger-fitness/coach/fixtures/
  program_current_cases.yaml` — ≥10 dated cases spanning block 0 start,
  block boundaries, light week 4, test week 8, a Sunday, pre-program
  date (error/null). Both test suites load THIS file and assert
  identical outputs. Drift = failing test on either side.

### Task A.3 — worker: new get_coach_context + YAML plumbing

ledger-mcp: fetch + parse coach/{program,phase,strategy}.yaml (yaml
parser dependency or minimal parser — check bundle size limits),
resolve current versions (skip pending), build the §3 JSON in the §3
ORDER, sized < 2,500 tokens (drop weekly_template detail outside
today, elide nutrition prose). this_week/last_week/flags_open read
from the program_status/coach_flags tabs (null-safe before B lands:
omit sections when tabs missing). Keep goals.md + metrics.md; include
routine.md under a "deprecated" heading this release. Tests with
fixture YAML. Deploy + live token count check (§6.6).

### Task A.4 — app: CoachBrain context update

CoachBrain fetches the three YAML files alongside goals/metrics (same
1h cache), runs the Dart resolver, injects a compact "PROGRAM SLICE"
section (same content as worker's program+phase+strategy JSON,
rendered as terse text) ahead of the docs; drops routine.md from the
default doc set but keeps fetching it this release behind the
deprecation note. Nightly (`coach_nightly.sh`) swaps routine.md for a
`tool/program_slice.dart --date` dump so briefings see the same slice.

### Task B.1 — nightly compute → program_status + coach_flags tabs

`~/repos/ledger/tool/program_status_update.dart`: full-history read
(reuse backtest plumbing), compute weekly metrics + flags, REPLACE-ALL
write of `program_status` tab (one row per ISO week: all §2.5 fields,
flat columns; per_lift as four column groups; deviations column empty
until phase C) and `coach_flags` tab (id, fired_on, evidence JSON,
action, acknowledged — preserve existing acknowledged values by id+
fired_on when rewriting). Non-ledger tabs (kaya_ascents pattern; the
app/sync never touch them). Wire into coach_nightly.sh BEFORE the
briefing prompt is assembled, and have the briefing prompt include the
current program_status row + open flags instead of raw-dump-only.

### Task B.2 — worker: get_program_status + context wiring

New tool get_program_status({weeks=8}) → markdown table of last n
program_status rows + their flags. get_coach_context now fills
this_week/last_week/flags_open from the tabs. Update tool descriptions
(briefing generation order per §3). Tests + deploy + end-to-end curl.

### Task B.3 — visibility (cheap, uses existing pattern)

`program_status.view.yml` + input.yml with `read_only: true` in
airledger-fitness so the weekly table is browsable in the app's
Read-only section. Push AFTER confirming column headers match B.1's
output.

### Task FINAL — docs + acceptance sweep

- §6 acceptance re-run end-to-end (backtest PASS output attached to
  the commit message or a docs/ note; get_coach_context live under
  2,500 tokens returning block 0 / week N / cut).
- CLAUDE.md (ledger): coach v4 architecture summary; airledger
  docs/integrations.md untouched; ledger-mcp README tool list.
- Final code review across the three repos' diffs.
