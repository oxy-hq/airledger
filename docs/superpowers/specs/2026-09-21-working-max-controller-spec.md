# Working-max controller — user spec (verbatim, 2026-09-21)

Extends 2026-09-20-coach-intent-layers-spec.md. User directive: execute
without confirmation prompts (except §5 seed confirmation happens IN THE
APP, and acceptance §7.1 trace is printed for later sanity-check).

ADAPTATION HEADER (decisions, same mapping philosophy as the parent
spec): working_max + readings = append-only sheet tabs (`working_max`,
`readings`) written by the Dart controller (nightly + app on-log);
controller implemented ONCE in Dart (lib/services/working_max.dart),
with a TS port of evaluate() in ledger-mcp for the log_rows response,
cross-checked by shared fixture traces (coach/fixtures/ pattern);
load_policy definitions live in coach/program.yaml (versioned);
prescriptions computed on demand (app + worker), not stored. e1RM
SOURCE OF TRUTH (user, 2026-09-21): the airlayer view schema measure —
add `max_e1rm_capped` measure (Epley, reps capped 12) to
strength_tracker.view.yml; program_metrics + this controller must match
that expression; metrics.md is descriptive only, not authoritative.

## 0. Three numbers never to confuse
estimated max = measurement (best Epley, reps<=8, trailing 42d, parent
spec §2.5) — app-derived. working max = control setting percentages
hang off — set by controller from top-set RPE readings / test week /
manual override; versioned. test single = the RPE-8 single in week 8
(reading kind=test). Working max may be wrong by a few pounds; quick to
go down, steady to go up (a high working max turns 4x3s into RPE-8
triples — the 2024-25 fatigue pattern).

## 1. Data model
1.1 working_max (per lift, per variant, append-only): lift, variant,
value_lb, effective_from, source (seed|rule|test|manual|pain_cap),
reason, reading_id. get_coach_context returns current per lift + last
three changes.
1.2 reading (one per top set; created when a strength row on a main
lift has rpe set and is the day's heaviest of that lift, or the
template says top set): date, lift, variant, weight_lb, reps, rpe,
kind (heavy_top|saturday_single|test|light_week|capped — from week
type + day template), grinder (rpe>=9.5 or notes match
/grind|slow|stall|miss/), missed (reps < prescribed), implied_max
(weight / chart[rpe][reps]). Only heavy_top raises; saturday_single +
capped can lower; light_week recorded+ignored; test resets.
1.3 RPE chart (% of max by reps, RPE); outside table fall back to
inverted Epley pct = 1 / (1 + (reps + (10 - rpe)) / 30):
RPE 10:  1:100  2:95.5 3:92.2 4:89.2 5:86.3 6:83.7 8:78.6
RPE 9.5: 1:97.8 2:93.9 3:90.7 4:87.8 5:85.0 6:82.4 8:77.4
RPE 9:   1:95.5 2:92.2 3:89.2 4:86.3 5:83.7 6:81.1 8:76.2
RPE 8.5: 1:93.9 2:90.7 3:87.8 4:85.0 5:82.4 6:79.9 8:75.1
RPE 8:   1:92.2 2:89.2 3:86.3 4:83.7 5:81.1 6:78.6 8:73.9
RPE 7.5: 1:90.7 2:87.8 3:85.0 4:82.4 5:79.9 6:77.4 8:72.3
RPE 7:   1:89.2 2:86.3 3:83.7 4:81.1 5:78.6 6:76.2 8:70.7
RPE 6.5: 1:87.8 2:85.0 3:82.4 4:79.9 5:77.4 6:75.1 8:69.4
RPE 6:   1:86.3 2:83.7 3:81.1 4:78.6 5:76.2 6:73.9 8:68.0
1.4 Variants (parse from notes via keywords paused/pins/touch and go/
belted/unbelted/straps; unknown -> default; show parsed variant in app
for correction): bench default paused (touch_and_go +3% -> divide by
1.03 before comparing; pins = paused). squat default belted (unbelted
-3%; paused -3%; "belted, paused" = -3% once). deadlift default belted
(unbelted -4%; straps/double-overhand no change). press standard.
1.5 load_policy (the one abstraction; per phase/week type): name,
target_rpe, raise_if_rpe_lte, hold_band, drop_if_rpe_gte, step_lb
{bench 5, press 5, squat 5, deadlift 5}, frozen, cap_rpe,
cap_after_drop_rpe, consecutive_drops_action (no_top_sets_next_week),
readings_that_raise [heavy_top], readings_that_lower [heavy_top,
saturday_single, capped], reset_on test (-> wm = weight/0.922 rounded
DOWN to 5).

## 2. Controller (after every reading-creating log_rows, and nightly)
evaluate(lift): policy from program.current(today); r = latest reading
since last evaluation (converted to default variant); wm = current.
No reading: two consecutive weeks without -> flag NO_READING; else
hold. r.kind test + reset_on test -> set(round_down_5(weight/0.922),
source test). light_week -> hold. grinder or missed or rpe >=
drop_if_rpe_gte -> wm - step, cap next top set at cap_after_drop_rpe;
consecutive drop -> consecutive_drops_action. frozen -> hold (only the
drop rule runs above). r.kind in readings_that_raise and rpe <=
raise_if_rpe_lte -> wm + step. else hold.
Overrides checked FIRST: PAIN_CAP (daily note cause=pain naming lift/
region: back->squat+deadlift, elbow/finger->press+bench: freeze +
cap_rpe 7 until two consecutive clean heavy sessions, source
pain_cap). TWO_SIGNALS flag this week -> all lifts frozen this week.
VARIANT_MISMATCH (no conversion) -> hold + deviation prompt. MANUAL
(set_working_max) -> applies immediately.
After every evaluation write a prescription for the next heavy session.

## 3. Policies by phase
cut_early (block 0 Sep 21-Nov 15 2026): target 7.5-8; raise if <=7
TWICE IN A ROW; hold 7.5-8.5; drop >=9; not frozen; cap 8.5.
cut_late (block 0 Nov 16-Dec 13): target 7; no raises; hold 6.5-7.5;
drop >=8.5; FROZEN; cap 7.
reverse (block 1): as cut_late; seed from last cut_late single / 0.892.
lifting_block (blocks 3,5,7): target 8.5-9; raise <=8; hold 8.5-9;
drop >=9.5; cap 9.
climbing_block (blocks 2,4,6): target <=8; no raises; drop >=9;
FROZEN; cap 8.
light_week (week 4): frozen, cap 6, readings ignored; week 5 resumes
at week 3's number.
test_week (week 8): frozen, cap 8; the single resets wm (/0.922, round
down 5).
Cut expectation: bodyweight falls ~0.75/wk; squat/deadlift drift 1-2%/
month; a drop or two is not failure. cut_early's twice-in-a-row raise
rule stops one good day pushing the number up in a deficit. From Nov
16 nothing rises; last RPE-7 singles seed the reverse.

## 4. Prescription (before a heavy session; advisory)
Per lift: policy name, wm (variant); warm-ups (existing protocol); top
set options weight = wm * chart[target_rpe][reps] for 1/2/3 reps;
back-offs 4x3 at 81-83%; Sat single at 8.5 (omit in cut_late/reverse/
climbing_block); last three readings with decisions. Round 5 (2.5
bench/press if microplates).

## 5. Seeds (Sep 21 2026, from RPE-logged top sets since Aug 1)
bench 240 paused (230x1@9 paused Sep 14 -> 241). squat 320 belted
(295x1@8 Sep 15 -> 320). deadlift 330 belted (315x1@8 Sep 11 -> 342;
325x1@9 Aug 28 -> 340; back twinge Sep 17 -> PAIN_CAP active at seed).
press 140 standard (120x5@8 Aug 22 -> 150; 105x5@7 Sep 14 -> 134;
median). User confirms all four IN THE APP before they go live.

## 6. MCP
get_coach_context += working_maxes [{lift, variant, value_lb,
effective_from, source, last_changes[3]}] + next_prescriptions (next
two heavy days). log_rows: when a row becomes a reading, run evaluate
and return {reading, decision raise|hold|drop|freeze|reset,
new_working_max?, reason, next_prescription}. set_working_max({lift,
value_lb, reason, user_quote}) manual override, applies immediately.
get_program_status weekly rows += working_max_end_of_week per lift +
the week's decisions.

## 7. Acceptance
1. Replay Aug 1 - Sep 21 2026 through cut_early with §5 seeds. Bench
   expected: Aug 17 230x1@8 hold; Aug 24 230x1@9 drop->235 cap 8.5;
   Sep 7 185x5@7 paused hold; Sep 14 230x1@9 paused drop->230. Print
   trace for user sanity-check.
2. light_week readings never change any wm.
3. test 240x1 -> bench 260; 245x1 -> 265.
4. touch_and_go bench reading divided by 1.03 before comparison.
5. Pain note containing "back" freezes squat+deadlift cap 7; two clean
   heavy readings lift it.
6. Two weeks no reading -> NO_READING.
7. Policy switch at block boundary never changes wm by itself.
