# Coach intent layers — user spec (verbatim, 2026-09-20)

Adaptation decisions approved 2026-09-20 (see plan doc): intent layer as
versioned YAML in airledger-fitness/coach/ (program.yaml, phase.yaml,
strategy.yaml; append-only versions lists); metrics as one pure-Dart
library writing non-ledger program_status + coach_flags tabs (nightly
Mac compute; MCP reads tabs); two program.current resolvers (TS worker +
Dart) cross-checked by shared fixtures; build order B0 (metrics+backtest
gate) -> A (intent) -> B (outcome) -> C -> D.

---

## 0. The problem this solves

Today the pipeline is: raw rows -> LLM -> advice. The rows record what
was done. They do not record what I intended, why I deviated, or whether
a change in my training was a decision or a symptom. A year-long
post-mortem of my 2025 bulk needed three facts the rows could not give:
was the May 2025 switch from heavy triples to 12-rep deadlift sets a
choice or exhaustion; was the falling volume in July 2024 a planned
wind-down or fatigue; was the cut that followed a plan or a reaction.
Each was a change of strategy that never got recorded as one.

The fix is three layers: Intent (versioned program + strategy note),
Execution (rows + daily note capturing the qualitative why), Outcome
(derived metrics computed in the app, not by the LLM). The coach
compares layers and reports mismatches; it proposes changes to intent;
only the human writes them.

## 1. Principles

- Intent is declared by the human, never inferred by the model.
- Deviations are captured when they happen.
- Derived metrics live in the app; the LLM reads them.
- Everything carrying intent is versioned (effective_from + reason).
- The coach proposes, the human confirms; no MCP tool silently
  overwrites strategy or phase.
- Free text stays primary; structured daily-note fields are optional.

## 2. Data model

### 2.1 program (versioned; one active)

id, version, effective_from, reason, blocks, week_types
(normal / light {week_in_block: 4} / test {week_in_block: 8}),
weekly_template (per weekday, per block emphasis), loads, targets,
rules, nutrition. get_coach_context returns the CURRENT SLICE via
program.current(date): block, week_in_block, week_type, today_template,
targets_in_force, rules_in_force.

### 2.2 phase (declared)

value (cut|reverse|bulk|maintain), effective_from, reason,
entry_criteria, exit_criteria, target_weight_lb,
target_rate_lb_per_week. The app never changes phase.value; it computes
whether the scale agrees (PHASE_MISMATCH).

### 2.3 strategy (versioned note)

One short present-tense paragraph written by the user; every edit is a
new version with effective_from + reason. Returned verbatim by
get_coach_context. House rule: any deviation from the program lasting
more than a week must be written into the strategy note.

### 2.4 daily_notes — three optional fields

adherence enum(as_planned, reduced, substituted, skipped, extra);
cause enum(fatigue, pain, time, illness, travel, choice, other);
readiness int 1-5. Never required.

### 2.5 Derived metrics (nightly + on every log_rows)

Main lifts by EXACT logged exercise name:
  Barbell Squat -> squat; Flat Barbell Bench Press -> bench;
  Barbell Deadlift -> deadlift; Overhead Press -> press;
  Barbell Standing Military Press -> press.
Variants (front squat, close-grip bench, sumo, stiff-leg, incline) are
accessories, NOT main lifts.

Per set (main lifts only):
  set_e1rm   = weight * (1 + min(reps, 12) / 30)        # Epley, cap 12
  reference  = max(set_e1rm) over same lift, sets with reps <= 8, in the
               42 days ending on and including this date; carry last
               known reference forward if none in window
  effort     = set_e1rm / reference
  pct_max    = weight / reference
  tier       = warm_up (<0.80) | moderate (0.80-0.90) | hard (>=0.90)
  working    = effort >= 0.80
  near_max   = effort >= 0.95 and reps <= 8
  long_failure_set = reps >= 8 and effort >= 0.95
Amendment 2026-09-20 (user-approved): near_max requires reps <= 8 — AMRAP long-failure sets must not silence NEAR_MAX_LOW; the 2025 backtest showed effort>=0.95 alone lets them mask missing heavy work. long_failure_set unchanged.

Per ISO week (Mon-Sun): sessions (distinct dates with a main-lift set),
sets_total, working_sets, hard_sets, near_max_sets, long_failure_sets,
avg_reps_working, per_lift {days, working_sets, near_max_sets,
best_e1rm_from_sets_le5}, bench_days, climbing_sessions (from climbing
snapshot; flag staleness), bike_4x4 {count, max_hr,
work_rate_or_speed}, bw_7d_avg (trailing 7-day mean of weight_lbs as of
Sunday), bw_rate_lb_wk (this Sunday minus last Sunday),
bw_3wk_change (this Sunday minus three Sundays ago), week_type,
deviations (section 4). Exposed as program_status weekly rows and, for
current + previous week, inside get_coach_context.

Amendment 2026-09-22 (user): the ACCOUNTING week's start day is
configurable via program.yaml `week_start` (v7: saturday — weekend
sessions read as getting ahead of the coming week, not catching up the
old one). Scope: weekly rollup keying only — the metrics above, flag
weeks, the app's live this-week strip, the planner's generation window,
and the weekly Wilks stat all key by weekStartOf(date, week_start);
"Sunday" in the bw definitions reads as "the week's last day". Program
STRUCTURE (block boundaries, week_in_block, week_type) stays
Monday-anchored; accounting weeks resolve onto it via the first Monday
on/after the week start (anchorMondayOf — the Monday owning the week's
Mon-Fri). The §6 backtest stays pinned to Monday weeks: its acceptance
numbers were validated against Monday keying and remain the gate.

### 2.6 Flags (weekly, Sunday night or on demand)

Each rule: id, condition, action, scope (week types). Seed:
  WEIGHT_FAST     bw_rate_lb_wk > 0.6 two consecutive weeks ->
                  "Take 100 kcal/day out now; don't wait for the
                  three-week check."
  WEIGHT_FLAT     phase=bulk and abs(bw_3wk_change) < 0.3 ->
                  "Add 100 kcal/day."
  WEIGHT_CAP      bw_7d_avg > 172 -> "Hold at maintenance until the
                  next block starts, whatever the block was for."
  WEIGHT_DRIFT    every third Sunday: bw_7d_avg > 1 lb from block target
                  line -> "-100 kcal if over, +100 if under. Never a
                  bigger step."
  PHASE_MISMATCH  bulk and rate <= -0.2 x3wk, or cut and rate >= +0.2
                  x3wk, or maintain and abs(bw_3wk_change) > 1.5 ->
                  "Declared {phase}; scale says otherwise for three
                  weeks. Food or declaration is wrong."
  NEAR_MAX_LOW    week_type=normal and near_max_sets < 5 -> "Heavy work
                  is missing. Every productive stretch had 6+ near-max
                  sets a week; every failed one had 4 or fewer."
  WORKING_LOW     week_type=normal and working_sets < 20 -> "Working
                  volume under 20. Target ~28."
  LONG_SETS       long_failure_sets >= 2 -> "Two or more long sets to
                  failure. This is the 2025 pattern; they replace heavy
                  sets, they don't add to them."
  BENCH_ONCE      week_type=normal and bench_days < 2 -> "Bench once
                  this week. Twice is the rule in every phase."
  TUESDAY_LOWER   any squat/deadlift set effort >= 0.85 on a Tuesday ->
                  "Heavy lower on a climbing day. Never."
  CLIMB_OVER      climbing_sessions > block allowance (2 lifting, 3
                  climbing blocks) -> "Climbing over the block's
                  allowance."
  BIKE_DROP       4x4 max_hr or work rate below median of previous four
                  4x4s, two weeks running -> "Take the light week
                  early."
  TOP_SET_HEAVY   two consecutive top sets on one lift at RPE >= 9.5 ->
                  "Next week: RPE 8 on that lift, no top-set attempts."
  PAIN_NOTE       a daily note with cause=pain this week -> "Check the
                  finger / elbow / back gates in the program."
  TWO_SIGNALS     two or more fired same week -> "This week becomes a
                  light week."
  BLOCK_END       week_type=test: compare four RPE-8 singles with
                  previous test. If flat: "Were the inputs delivered? If
                  yes, next lifting block at +0.4/week. If no, fix the
                  inputs and leave the food alone."
Flags carry fired_on, evidence, acknowledged; unacknowledged flags
reappear in the next briefing.

## 3. MCP changes

get_coach_context: return as_of, phase, strategy, program slice (id,
version, block {number, emphasis, dates, target_weight}, week_in_block,
week_type, today_template, targets_in_force, rules_in_force),
this_week, last_week, flags_open, goals.md, metrics_definitions,
views. Under 2,500 tokens. routine.md retired in favour of
program.weekly_template (readable one release, then removed).

get_program_status (new): {weeks: n} -> last n weekly rows + flags as
markdown. Default 8.

set_strategy / set_phase (new, phase D): pending versions with
source: coach_chat + verbatim user_quote; not returned by
get_coach_context until confirmed (in-app one-tap, or "confirm" reply
handled by the app). Never called without the user saying, in their own
words, that strategy/phase is changing.

add_daily_note: optional adherence/cause/readiness; model sets them
from the user's words when explaining a missed/reduced/substituted
session; never guess cause.

log_rows: after appending, evaluate section 4 vs today's template;
return deviations: [...]. Non-empty -> model asks one plain question and
records the answer via add_daily_note.

post_coach_message: no change. Nightly briefing (thread briefings)
generated from program_status + open flags: week type and place in
block; this week vs targets; flags with action text; unexplained
deviations each with its one question.

## 4. Deviation detection (every log_rows + nightly)

MISSING_LIFT (template lists lift, no working set by end of day);
EXTRA_LIFT (working sets on a lift not in template); NO_TOP_SET (heavy
day, no set with effort >= 0.93); LONG_SET (working set reps >=
template_reps + 3); LOWER_ON_TUESDAY (squat/deadlift effort >= 0.85 on
Tuesday); EXTRA_CLIMB (session on a non-template day or past block
allowance); MISSED_4X4 (Wednesday with no 4x4 row by Thursday night);
LIGHT_WEEK_VIOLATION (light week, any set effort >= 0.90).
Each: date, lift_or_activity, expected, observed, explained_by (daily
note id). Unexplained > 48h -> next briefing with its question.

## 5. Seed data (Bulk Program 2026-27 doc, Sep 20 2026)

### 5.1 Blocks
blocks:
  - { n: 0, dates: [2026-09-21, 2026-12-13], emphasis: cut,      weight: [163, 154], notes: "Maintenance week + DEXA Nov 2-8. Top singles at RPE 7 from Nov 16." }
  - { n: 1, dates: [2026-12-14, 2027-01-03], emphasis: reverse,  weight: [154, 155], notes: "Add ~150 kcal/day per week until the 7-day average holds two weeks; that is maintenance. Full training week at RPE 7." }
  - { n: 2, dates: [2027-01-04, 2027-02-28], emphasis: climbing, weight: [154, 157], rate: 0.4 }
  - { n: 3, dates: [2027-03-01, 2027-04-25], emphasis: lifting,  weight: [157, 160], rate: 0.4 }
  - { n: 4, dates: [2027-04-26, 2027-06-20], emphasis: climbing, weight: [160, 163], rate: 0.4, notes: "End with an outdoor trip or a comp." }
  - { n: 5, dates: [2027-06-21, 2027-08-15], emphasis: lifting,  weight: [163, 166], rate: 0.4 }
  - { n: 6, dates: [2027-08-16, 2027-10-10], emphasis: climbing, weight: [166, 168], rate: 0.2 }
  - { n: 7, dates: [2027-10-11, 2027-12-05], emphasis: lifting,  weight: [168, 170], rate: 0.2, notes: "Then hold 168-170." }
hard_cap_lb: 172
band_lb: [154, 172]
Blocks 2-7 are 8 weeks: weeks 1-3 normal, week 4 light, weeks 5-7
normal, week 8 test.

### 5.2 Weekly template (blocks 1-7)
Mon AM: Squat heavy: top set of 1-3 at RPE 8.5-9, then 4x3 at 82%;
  hanging leg raise; muscle-ups (3-4 sets of 1-2, 2x3-5 banded strict,
  front lever up-downs 2x5, ~10 min).
Tue AM: Bench heavy: top set of 1-3, then 4x3 at 82%; press light 3x8
  at 70%. PM: Climb 1, limit/projecting (partner day, through June
  2027). No squat or deadlift on this day.
Wed AM: Bike 4x4; 10 min hip and thoracic mobility. Easy day.
Thu AM: Deadlift heavy: top set of 1-3, then 3x3 at 82%; press heavy:
  top set, then 4x3.
Fri PM: Climb 2, board/steep-wall power. Max hangs first in climbing
  blocks.
Sat AM: Bench: one single at RPE 8.5, then 3x8 at 70%; weighted
  pull-ups 5x3, rows, face pulls; squat: one single at RPE 8.5, then
  3x8 at 70%; optional deadlift 3x5 at 65% only after four clean weeks.
  Longest session.
Sun: Off. PM: Climb 3, volume/technique, climbing blocks only.
Block 0 (cut): current routine plus a second bench day (light 3x5 at
70%), climbing Tuesday afternoons, no heavy lower on Tuesdays, top
singles at RPE 7 from Nov 16.

### 5.3 Loads
top_set: reps 1-3, rpe 8.5-9, pct_max_by_reps {1: 0.96, 2: 0.90,
  3: 0.88}, rotate_weekly. back_offs: reps 3, sets {squat 4, bench 4,
  press 4, deadlift 3}, pct_max 0.81-0.83, rpe 6.5-7.5. volume_day:
  3x8 at 0.70, "add one rep per set weekly to 3x10, then +5 lb and back
  to 3x8". saturday_single: bench+squat at RPE 8.5. progression: "+5 lb
  to working max when top set RPE 8.5 or easier two sessions running".
  regression: "two consecutive top sets at RPE 9.5 -> next week RPE 8,
  no top-set attempts on that lift". never: "no grinders". light_week:
  top_set_rpe 6, back_offs 0, volume 2x8, climbing one fewer, hangboard
  off. test_week: "one single at RPE 8 on squat, bench, deadlift,
  press; those four numbers are the block's result". climbing_block:
  top_set_rpe_cap 8, back_offs held, climbing 3, max hangs 5x7s on
  20mm before the two hard sessions. lifting_block: climbing 2, both
  below limit, hangboard once a week or off.

### 5.4 Targets
gain_rate_lb_wk: blocks_2_5 [0.2, 0.5], blocks_6_7 [0.1, 0.3],
alarm 0.6. bodyweight_lb: band [154, 172], hold [168, 170].
near_max_sets_wk 6; working_sets_wk 28 (never below 20 normal);
bench_days_wk 2; press_days_wk 2; squat_days_wk 2; deadlift_days_wk 1;
climbing_wk {lifting_block 2, climbing_block 3}; bike_4x4_wk 1;
muscle_up_sessions_wk 1; block_length_wk 8; protein_g_per_lb [0.8,1.0].

### 5.5 Nutrition
surplus_kcal {rate_0_4: 200, rate_0_2: 100}. adjust: "every third
Sunday, 7-day average vs block target line: >1 lb over -> -100; >1 lb
under -> +100; never a bigger step". maintenance_week: "week 4 of every
block at maintenance". carbs: "extra on the four lifting days and the
limit-climbing day". maintenance_definition: "the intake at which the
7-day average holds for two weeks; recorded at end of block 1".

### 5.6 Phase seed
value cut; effective_from 2025-10-06; reason "Ending the 2025 bulk at
186.5; the extra weight bought nothing"; target_weight_lb 154;
exit_criteria "DEXA week of Nov 2 at ~157 lb sets the real endpoint
(13%); maintenance week Nov 2-8; cut ends Dec 13 2026; reverse diet
starts Dec 14".

### 5.7 Strategy seed (Robert edits before save)
Block 0, cut, week 1 of 12. 7-day average 163; target 154 by Dec 13,
~0.75 lb/week. Lifting four days with top sets at RPE 7-8 (RPE 7 only
from Nov 16), bench twice a week from this week. Climbing Tuesday
afternoons with a partner plus one more session; no heavy squat or
deadlift on a Tuesday. Deadlift is unbelted and held back after the
back twinge on Sep 17 until two clean weeks. Muscle-ups Mondays after
squats, maintenance dose only. One bike 4x4 on Wednesdays. Nothing else
is being pushed; the point of this block is to arrive at 154 with the
lifts intact.

## 6. Backtest before shipping (acceptance)

Run 2.5 metrics + 2.6 flags over full history and check:
1. NEAR_MAX_LOW fires weeks of May 12, May 19, May 26, Jun 9 2025;
   LONG_SETS fires in at least three of them; NEAR_MAX_LOW does NOT
   fire in any week of Apr 22 - Jun 24 2024.
2. WEIGHT_FAST fires at least twice between Apr 21 and Jun 22 2025.
3. working_sets Apr 29 - Jul 28 2024 averages 22-26; Apr 21 - Jun 22
   2025 averages 12-16.
4. near_max_sets Dec 2 2024 - Feb 10 2025 averages 12-16.
5. RPE sanity: sets with logged RPE, by effort band, average within
   +-0.4 of (0.85-0.90 -> 7.3, 0.90-0.95 -> 7.8, >=0.95 -> 8.5).
6. get_coach_context today: block 0, week 1, type normal, cut phase,
   under 2,500 tokens.
If 1-4 don't reproduce, the effort grading is wrong; fix before
building on top.

## 7. Sequencing

A intent layer (program/phase/strategy storage + seed + current slice
in get_coach_context; retire routine.md). B outcome layer (effort
grading, weekly metrics, program_status view, flags, backtest, nightly
job). C collection layer (daily-note fields; log_rows deviations + one
question; briefing rewrite). D confirmation (set_strategy/set_phase
pending writes; flag acknowledgement). Ship A and B together.
