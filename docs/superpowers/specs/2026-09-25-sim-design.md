# Program-simulation design — sim core, world model, adaptive phases (W1 output)

Companion to `2026-09-25-program-simulation-spec.md` (the user directive)
and `2026-09-25-sim-calibration-study.md` (fitted coefficients +
walk-forward validation; regenerate with `dart run tool/sim_calibrate.dart
--out <path>` in the ledger repo). This doc fixes the W2 contract: state
vector, week-step equations, adaptive phase rules, lever plumbing, the
declared world-model config, nightly refit, the W3 UI sketch, and the W2
acceptance gate.

## 1. Airlayer world-model findings (spec §5 investigation)

Airlayer (~/repos/airlayer) **does** have a first-class drivers concept:

- **Shape**: measures in view YAML declare `drivers:` — a list of
  `{measure: "view.measure", direction: positive|negative|unknown,
  strength, confidence, coefficient, form: linear|log_log|log_linear|
  linear_log, intercept, lag (days), description, refs}`
  (`src/schema/models.rs` `Driver`, docs/schema-format.md §Drivers).
- **Semantics**: driver edges (`EdgeKind::Driver`) join component edges
  in the metric tree (`src/engine/metric_tree.rs`); `airlayer inspect
  --metric-tree` renders them; `inspect --json` emits an entity-first
  `ontology` block explicitly framed as "the view a world model consumer
  ingests" (docs/promotions.md §World model ontology lift;
  examples/two-plane-rca is the demo that "fails without a world model").
- **Why we don't host the graph there (yet)**: the ledger app's embedded
  airlayer surface (sdk-dart FFI: `compile`/`validate`/catalog/cache
  keys) does **not** expose the metric tree or driver annotations, and
  `AnalyticsEngine.viewYamlForAnalytics` synthesizes view YAML on the
  fly from the app's `ViewSchema`, which has no drivers field — the app
  would need new Dart-side parsing either way. Additionally airlayer's
  `Driver` carries ONE coefficient slot per edge: no per-phase
  conditioning, no fit metadata (window, n, R², MAE), no saturation
  rule — all load-bearing here.

**Decision**: declare the causal graph in
`~/repos/airledger-fitness/app/world_model.yaml` (same delivery path as
`app/dashboards.yaml`: assets + SchemaSync), **adopting airlayer's
driver vocabulary verbatim** (`measure` refs, `direction`, `form`,
`coefficient`, `intercept`, `lag`) and extending it with `fit:` metadata
and `sim:` rules. A future lift into airlayer proper is then mechanical
(strip the extensions, hang the drivers on the measures). Code only
evaluates the declared config — consistent with the repo's
schema-driven ethos.

## 2. Simulator state vector

Weekly state `S(t)`, `t` = Monday-keyed ISO week:

| field | unit | source at t=0 (current observed state) |
|---|---|---|
| `bw` | lb | 7-day mean of latest week's weigh-ins (carried) |
| `e1rm[squat, bench, deadlift, press]` | lb | weekly best RPE-adjusted e1RM (`rpeAdjustedE1rm`, reps capped 12), carried |
| `peak[lift]` | lb | running all-time max of `e1rm[lift]` (history ∪ sim) |
| `grade_p75` | V | rolling 4-week p75 of numeric-V kaya ascents (>= 5 ascents), carried |
| `phase` | enum | cut / reverse / bulk / maintain, from the phase schedule |
| `wilks` | — | DERIVED: `wilksPointsLb(k · Σ_sbd e1rm, bw)` where `k` = observed actual-max SBD total / e1RM SBD total at t=0, held constant (basis anchor) |

Reference values 2026-09-21 (from the study's state-vector table):
bw 160.6; e1RM squat 311.7 / bench 247.5 / deadlift 351.8 / press 144.0;
Wilks (actual-max) 321.2; grade p75 V5; climb 2 sessions/wk.

## 3. Week-step equations (the sim core, pure Dart — W2)

Bodyweight is the **controlled input**, scripted by the phase schedule
and levers, not a fitted response:

```
bw(t+1) = bw(t) + rate(phase(t))        // cut_rate < 0, bulk block rate,
                                        // reverse +0.15, maintain 0
```

Strength — model **S4** from the study (per-lift bw-rate response with
peak-saturation damper; heavy-exposure spec S1 rejected, it compounds
catastrophically over long horizons):

```
v_raw[l](t)  = a[l] + b_bw[l] · rate(phase(t))
damp[l](t)   = clamp((1.05·peak[l] − e1rm[l](t)) / (0.10·peak[l]), 0, 1)
v[l](t)      = v_raw > 0 ? v_raw · damp : v_raw       // damper on gains only
e1rm[l](t+1) = e1rm[l](t) + v[l](t)
peak[l](t+1) = max(peak[l](t), e1rm[l](t+1))
```

Because `peak` updates with the sim, the damper asymptotes to 0.5× at
the all-time frontier — "an advanced trainee at the ceiling gains at
half the fitted response" — which is what answers "do I gain strength
indefinitely?" honestly: cycles keep adding, at a decaying rate, and
cuts (negative `v_raw`) pass through undamped.

Fitted coefficients (full history, 2011–2026, 4-wk blocks, bw-only spec;
see study for R²/MAE per fit):

| lift | a (lb/wk) | b_bw (lb/wk per lb/wk) |
|---|---|---|
| squat | −0.280 | 2.295 |
| bench | −0.099 | 0.919 |
| deadlift | +0.775 | 0.795 |
| press | −0.143 | 0.352 |
| pooled | +0.072 | 1.100 |

Climbing — model **C2** (level-anchored; the integrated velocity model
C1 drifts unboundedly):

```
grade_p75(t) = c0 + c_bw · bw(t) + b_f · (climb_freq − 2) · min(t, 26)
             // c0 = 8.56, c_bw = −0.0288 V/lb  (n=108 wk, MAE 0.54 V)
             // b_f = 0.0032 V/wk per session/wk (C1's frequency term):
             // a small, capped (~6 months) adjustment so the frequency
             // lever does something, honestly bounded
```

Uncertainty bands: render forecast ± the walk-forward MAE of the
matching output/phase from the study (§7 gate table) — calibration
quality is first-class in the UI per the spec.

## 4. Adaptive phase rules

Sim always starts from the CURRENT observed state (t=0 = latest week).
Phase schedule is generated, then re-generated on every re-sim:

1. **Cut ends at target-or-date** — whichever comes first:
   `bw <= target_weight_lb` (154, coach/phase.yaml v1) OR the declared
   end date (2026-12-13, program v7 block 0). Lighter-now ⇒ the target
   trips earlier ⇒ shorter cut (the user's example, by construction).
2. **Early cut end inserts maintain filler**: program blocks 1–7 keep
   their declared calendar dates (emphases are season/trip-anchored);
   the gap between (cut end → block 1 start) fills with `maintain` at
   rate 0. Reverse (block 1) behaves as maintain-to-slight-gain
   (+0.15 lb/wk).
3. **Bulk blocks keep dates, rate is a lever**: blocks 2–5 default
   0.4 lb/wk, 6–7 default 0.2 (program v7 `blocks[].rate`); the bulk
   lever scales all of them.
4. **Next-cycle auto-generation** (the ~3-year horizon): after the
   program's last block (2027-12-05, hold band 168–170): hold
   `maintain` until `bw >= band_top (170)` would be exceeded — with
   maintain rate 0 that is immediate, so: hold 8 weeks (one block), then
   cut at `cut_rate` to the same cut target (154), reverse 3 wk, bulk at
   the lever rate back to band top, repeat until the horizon. Each
   generated cut obeys rule 1 (target-or-date with date = start +
   ceil((bw − target)/|cut_rate|) weeks).

## 5. Levers (instant client-side re-sim)

| lever | range | default |
|---|---|---|
| bulk gain rate | 0.1 .. 0.6 lb/wk | program block rates (0.4/0.2) |
| cut rate | −1.6 .. −0.5 lb/wk | −0.75 (phase.yaml target) |
| climbing frequency | 2 or 3 /wk | 2 (strategy v1 cap) |
| horizon | 1 .. 3 yr | through next full cycle (~2028-12) |

Levers re-run the pure-Dart sim synchronously (hundreds of weeks × 4
lifts — trivial); no persistence, no network.

## 6. Where coefficients live — `app/world_model.yaml` (proposed content)

Parsed by the app (new `lib/services/world_model.dart`, yaml package,
same pattern as `parsePhaseEigenvectors`); synced via SchemaSync like
dashboards.yaml — **remember to push airledger-fitness** (trap #2).
Proposed initial file:

```yaml
# app/world_model.yaml — declared causal graph + fitted response models
# for the program simulator. Driver vocabulary matches airlayer
# (docs/schema-format.md §Drivers): measure/direction/form/coefficient/
# intercept/lag. Extensions: fit metadata, saturation, sim rules.
# Coefficients are REFIT from history (tool/sim_calibrate.dart fit code
# shared with the app's SimRefit); the values here are the shipped
# defaults + the audit trail of the last refit.
version: 1
nodes:
  - id: bw            # lb, controlled input (phase schedule + levers)
    measure: weight.bw_7d_avg
    kind: input
  - id: e1rm_squat    # lb, RPE-adjusted weekly best
    measure: strength_tracker.max_e1rm_capped
    filter: { lift: squat }
  - id: e1rm_bench
    measure: strength_tracker.max_e1rm_capped
    filter: { lift: bench }
  - id: e1rm_deadlift
    measure: strength_tracker.max_e1rm_capped
    filter: { lift: deadlift }
  - id: e1rm_press
    measure: strength_tracker.max_e1rm_capped
    filter: { lift: press }
  - id: grade_p75     # numeric V, rolling 4-wk p75
    measure: kaya_ascents.grade_p75
  - id: wilks         # derived: wilks(k * sbd_total, bw)
    kind: derived

drivers:
  # strength velocity (lb/wk) = intercept + coefficient * d(bw)/dt
  # damped by saturation on the gain side (sim.saturation).
  - target: e1rm_squat
    driver: bw
    form: linear_rate          # responds to the DRIVER'S weekly rate
    direction: positive
    coefficient: 2.295
    intercept: -0.280
    fit: { method: ols_4wk_blocks, window: 2011-05..2026-09, n: 88, r2: 0.06, mae_lb_wk: 5.00 }
  - target: e1rm_bench
    driver: bw
    form: linear_rate
    direction: positive
    coefficient: 0.919
    intercept: -0.099
    fit: { method: ols_4wk_blocks, window: 2011-05..2026-09, n: 86, r2: 0.01, mae_lb_wk: 3.91 }
  - target: e1rm_deadlift
    driver: bw
    form: linear_rate
    direction: positive
    coefficient: 0.795
    intercept: 0.775
    fit: { method: ols_4wk_blocks, window: 2011-05..2026-09, n: 94, r2: 0.01, mae_lb_wk: 5.60 }
  - target: e1rm_press
    driver: bw
    form: linear_rate
    direction: positive
    coefficient: 0.352
    intercept: -0.143
    fit: { method: ols_4wk_blocks, window: 2011-05..2026-09, n: 90, r2: 0.01, mae_lb_wk: 2.17 }
  # climbing grade tracks bw LEVEL (level-anchored — integrated
  # velocity models drift; see calibration study)
  - target: grade_p75
    driver: bw
    form: linear               # level-on-level
    direction: negative
    coefficient: -0.0288
    intercept: 8.56
    fit: { method: ols_weekly_level, window: 2025-05..2026-09, n: 108, r2: 0.07, mae_v: 0.54 }
  - target: grade_p75
    driver: climb_frequency
    form: linear_rate
    direction: positive
    coefficient: 0.0032        # V/wk per session/wk, capped 26 wk in sim
    fit: { method: ols_4wk_blocks, window: 2025-05..2026-09, n: 20, r2: 0.09, mae_v_wk: 0.126 }

sim:
  saturation:                  # S4 damper, gains only
    full_below_pct_of_peak: 0.95
    zero_at_pct_of_peak: 1.05
  phase_rules:
    cut_ends_at: target_or_date
    early_cut_filler: maintain
    reverse_rate_lb_wk: 0.15
    next_cycle: { hold_weeks: 8, band_top_lb: 170, cut_target_lb: 154, reverse_weeks: 3 }
  levers:
    bulk_rate_lb_wk: { min: 0.1, max: 0.6 }
    cut_rate_lb_wk: { min: -1.6, max: -0.5 }
    climb_frequency: [2, 3]
    horizon_yr: { min: 1, max: 3 }
  wilks_anchor: actual_max_ratio   # k = actual-max SBD total / e1RM total at t0
```

(The `measure:` refs are documentary today — the app computes these
series in Dart services, not through airlayer queries; when the FFI
grows an inspect binding the refs become live.)

## 7. W2 — sim core + acceptance gate

Deliverable: `lib/services/sim_core.dart` (pure Dart, no Flutter/IO) +
`lib/services/world_model.dart` (config parse) + fit code extracted
from `tool/sim_calibrate.dart` into `lib/services/sim_fit.dart` so the
app and the tool share one implementation (tool becomes a thin shell:
sheets IO + markdown). TDD; fixtures frozen from the study run.

**Gate (must reproduce the study's S4/C2 walk-forward table to within
0.1 from the same frozen weekly-series fixtures, and every model column
must still beat-or-tie the relationships below):**

| window | output | flat MAE | sim (S4/C2) MAE | must |
|---|---|---|---|---|
| 2024 bulk | squat e1RM | 22.2 | 16.1 | beat flat |
| 2024 bulk | bench e1RM | 10.9 | 14.7 | reproduce |
| 2024 bulk | deadlift e1RM | 21.0 | 23.4 | reproduce |
| 2024 bulk | press e1RM | 7.7 | 7.5 | beat flat |
| 2024 bulk | Wilks (derived) | 22.7 | 20.1 | beat flat |
| 2025 bulk | squat e1RM | 24.2 | 28.1 | reproduce (documented mid-bulk collapse) |
| 2025 bulk | bench e1RM | 14.9 | 15.9 | reproduce |
| 2025 bulk | deadlift e1RM | 26.7 | 41.3 | reproduce |
| 2025 bulk | press e1RM | 8.8 | 9.6 | reproduce |
| 2025 bulk | Wilks (derived) | 27.4 | 38.1 | reproduce |
| current cut | squat e1RM | 37.9 | 27.7 | beat flat |
| current cut | bench e1RM | 13.0 | 11.8 | beat flat |
| current cut | deadlift e1RM | 118.1 | 93.7 | beat flat (pain-cap regime, both huge) |
| current cut | press e1RM | 47.3 | 54.6 | reproduce |
| current cut | Wilks (derived) | 49.6 | 45.9 | beat flat |
| current cut | grade p75 | 0.71 | 0.62 | beat flat |

Directional acceptance (spec §2): the 2025 bulk and current cut
reproduce **directionally** (strength up in bulk, bw down + strength
held in cut) and the aggregate outputs (Wilks, and grade where data
exists) beat flat on the 2024 bulk and current cut. Plus: all existing
baselines hold (`flutter test`, 7 known failures only).

## 8. Nightly refit + surfacing (W3 wiring)

- `tool/program_status_update.dart` gains a sim step (or a sibling
  `tool/sim_refit.dart` called from `coach_nightly.sh` right after it):
  refit coefficients from full history (shared `sim_fit.dart` code), run
  the sim from current state with default levers, write:
  - **`world_model` tab**: one row per driver — target, driver, form,
    coefficient, intercept, fit window, n, R², MAE, refit timestamp.
  - **`forecast` tab**: one row per forecast week — date, phase, bw,
    per-lift e1RM, wilks, grade_p75, band_low/band_high.
  MCP (`get_recent_data`/`get_coach_context`) and the nightly briefing
  can then cite "model says X by March" with provenance.
- The app refits **on demand** (pull-to-refresh on the Program tab; and
  opportunistically once per day on open): same fit code over the local
  ledger, in-memory; world_model.yaml's shipped coefficients are the
  cold-start/fallback values and the declared graph shape. No git
  round-trip needed for freshness.
- Coefficient drift guard: if a refit coefficient leaves ±50% of the
  YAML default, the UI shows a "recalibrated" tag on the model card
  (and the nightly writes it to the world_model tab) rather than
  silently absorbing it.

## 9. W3 UI sketch (Program tab)

Top-to-bottom:

1. **Configuration + phases** (exists): program version, block list,
   phase chip — unchanged.
2. **Forecast section** (replaces the observed-duplicating parts): the
   observed weight chart stays as the anchor; the forecast extends it —
   observed line solid, simulated line dashed, phase bands as tinted
   vertical spans (cut/reverse/bulk/maintain), next-cycle bands lighter.
   Second track: strength — per-lift e1RM (selectable) + Wilks overlay,
   with ±MAE band. Third track: climbing grade p75 step-line with band.
3. **Lever row**: two sliders (bulk rate, cut rate), a 2/3 frequency
   toggle, horizon segmented control; re-sim on drag (sync, no jank at
   ~150 simulated weeks).
4. **Model card** (calibration first-class, spec §2): one line per
   model, e.g. "strength: +1.1 lb/wk per +1 lb/wk bw (squat 2.3), MAE
   16–28 lb across 3 held-out phases; climbing: −0.29 V per 10 lb, MAE
   0.6 V; refit nightly — tap for the study table."
5. Milestone readouts under the chart: cut-end date (adaptive), bw at
   each block boundary, projected Wilks at next bulk end, projected
   grade at cut end.

## 10. Open questions for the user (not blockers)

- Should the next-cycle cut target stay 154 lb or track a body-fat
  estimate once DEXA lands (Nov 2026)? Sim defaults to 154.
- Block dates for cycles beyond 2027: season-anchored (current design
  generates fixed-length blocks) or calendar-shifted copies of the 2027
  program?
- Press shows a large current-cut decline the maintain-model doesn't
  predict (MAE 47–55 both model and flat) — worth a strategy.md note or
  a press-specific exposure floor?
