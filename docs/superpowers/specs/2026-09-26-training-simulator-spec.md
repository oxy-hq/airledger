# Training simulator v2 — user-authored spec (moved verbatim, 2026-09-26)

> **Adaptation header (Claude, 2026-09-26).** This v2 spec was written by the
> user and SUPERSEDES the model layer of `2026-09-25-sim-design.md` (the
> overnight v1: S4 bw-rate strength model, C2 level-anchored climbing model,
> `world_model.yaml` coefficients). v1's UI/plumbing (Program-tab forecast
> section, world_model.yaml delivery via SchemaSync, nightly refit/forecast
> tabs) is to be adapted to this model in the next wave. Calibration CSVs
> referenced below live in the ledger repo at `tool/calibration/`; the wave-1
> fit/replay/horizon tools are `tool/sim2_fit.dart`, `tool/sim2_replay.dart`,
> `tool/sim2_horizon.dart`. Convergence note: v1's deterministic end-2027
> total was 967; this spec's §9.3 expectation is 980 (940–1020).
> Everything below this line is the user's spec, verbatim.

# Air Ledger: the training simulator ("program" view)

Spec for Claude Code. Extends `ledger-program-spec.md` and `ledger-working-max-controller.md`. Goal: a weekly-step model of how my four outputs respond to five dials, calibrated on my own log where the log can speak and on published anchors where it can't, so the program view can run "what if I climb 4× a week" or "what if I drop cardio" and show the effect on next year's strength, climbing grade, VO2 max and body composition.

Two files ship with this spec for calibration: `calibration_windows.csv` (174 overlapping 8-week windows, 2016–2026, inputs and the strength change over the following 14 weeks) and `calibration_weekly.csv` (every week since 2014: lifting sessions, sets, working sets, near-max sets, light-week flags, 7-day bodyweight, climbing sessions from Aug 2024).

## 0. Shape of the machine

**Outputs (state, updated weekly)**

| symbol | meaning | now (Sep 26 2026) |
|---|---|---|
| `S` | strength: squat+bench+deadlift estimated total, lb (per-lift split below) | 878 (squat 310, bench 238, deadlift 336; press 138) |
| `C` | climbing ability, continuous V grade | 6.6 (V7 once, V6 regular) |
| `VO2` | VO2 max score, ml/kg/min = `Vabs / BW_kg` | 52 |
| `BW`, `FM`, `LM` | bodyweight, fat mass, lean mass, lb | 163, ~29, ~134 (DEXA basis; Withings reads 5–6 points low) |
| `F` | fatigue, 0 = fresh, 1 = the 2025 pattern | ~0.3 |
| `M` | muscle-up capacity, clean reps at the Aug 19 2026 standard | 2 |

**Dials (set per block, or per week)**

| symbol | dial | unit | current plan (lifting block) |
|---|---|---|---|
| `N` | near-max sets per week (within 5% of best, RPE 8.5+) | sets | 6 |
| `W` | working sets per week on the four lifts (80%+ of best) | sets | 28 |
| `D` | lifting sessions per week | sessions | 4 |
| `K`, `K_lim` | climbing sessions per week, of which limit sessions | sessions | 2 (climbing block: 3), 1 limit |
| `H` | hangboard on (max hangs before hard sessions) | 0/1 | 0 (climbing block: 1) |
| `Z` | 4×4 bike sessions per week | sessions | 1 |
| `Z2` | zone-2 minutes per week | minutes | 0 |
| `Q` | calisthenics sessions per week (muscle-ups, front lever) | sessions | 1 |
| `r` | target bodyweight rate | lb/week | +0.4 (blocks 2–5), +0.2 (6–7) |
| `P` | protein | g per lb | 0.9 |

**One shared budget.** Every dial spends recovery. Under the budget the dials are close to independent; over it, everything's stimulus gets multiplied down and fatigue accumulates. That single mechanism is what turned 2025's extra climbing into stalled lifts, and it's why climbing at 2 sessions a week shows no relationship to strength gains in my log (windows with 1.9 climbs/week sit in both the best and worst thirds) while near-max sets do (3.9 in the worst third, 6.4 in the best).

## 1. Recovery budget and fatigue

```
load L  = 1.0·D + 0.15·max(0, N − 4)            # lifting sessions; heavy work costs a little extra
        + 1.0·(K − K_lim) + 1.4·K_lim            # limit climbing costs more
        + 0.7·Z + 0.3·(Z2/60)
        + 0.3·Q
capacity L_cap = 7.0 in a surplus, 6.0 in a deficit, 5.5 when BW > 176 (nothing good has happened above 178)
over    = max(0, L − L_cap) / L_cap
F_next  = 0.7·F + over                            # fatigue decays ~30%/week, builds when over budget
          F_next = 0.5·F_next in a light week
e       = 1 / (1 + 1.5·F)                         # effectiveness multiplier applied to every stimulus below
```

Anchors: Bulk C (4.5 lifting, 0 climbing, ~1 cardio) ≈ 5.2 units → under budget, F stayed low; Dec 2024–Feb 2025 (3.6 lifting + ~2 climbing) ≈ 5.6 → under; Bulk D weeks 11–19 (4.3 lifting + 3–4 climbing) ≈ 7.5–8.3 → over, and that is when the lifts stalled. The two-signal light-week rule in the program is the real-world version of the `F` reset.

## 2. Strength: capacity and expression are two different things

The log measures **expressed** strength (what showed up on the bar). Most of what moves week to week in a cut or the first weeks of a bulk is not muscle; it is glycogen and water, body mass as leverage, readiness, and whether near-max sets were even attempted. That is why strength falls fast in a deficit and comes back fast in a surplus: the fast part is expression, the slow part is capacity. Model them separately or every cut looks like lost muscle and every bulk's first month looks like a miracle.

```
S_obs = S_cap · E                                  # what the bar shows = capacity × expression

E = 1
  − 0.06 · dep         # energy state: dep ramps 0→1 over 3 weeks in a deficit, decays 1→0 over 3 weeks in a surplus
  + 0.0025 · (BW − 165)   # body mass as leverage: +2.5% per 10 lb (bench most, deadlift least; split 0.35/0.25/0.15)
  − 0.04 · rust        # rust ramps 0→1 over 4 weeks with N < 2, clears over 2 weeks of N ≥ 4
  − 0.03 · F           # fatigue
```

Anchors for `E`: RPE-based maxes today (bench 225×1 @8 → 244, squat 305 @8 → 331, deadlift 315 @8 → 342; total ≈ 917) sit 3–4% above the app's Epley index (878–889), which is what a deficit plus few near-max attempts does to a measure built from whatever sets happened. Dec 2024 → Mar 2025 gained +55 on the index in 14 weeks from a post-cut start with modest training change; Bulk C gained ~+34 in 7 weeks from a neutral start. The difference between those two is mostly `E` snapping back.

Capacity moves slowly and only from training, food and time:

```
g_N = a · (1 − exp(−N / 4))                       # near-max sets, saturating; a ≈ 2.5
g_W = b · (W − 20) / 10                            # working volume around 20; b ≈ 0.4
g_r = c · clip(r, 0, +0.5)  − c_cut · max(0, −r)  # food: c ≈ 1.5 in a surplus (nothing above +0.5/wk); c_cut ≈ 1.0 (lower volume, some lean loss)
d   = 0.6                                          # drift with no stimulus
pen = 0.5 if BW > 176 else 0
ΔS_cap = e · (g_N + g_W) + g_r − d − pen           # lb of total per week
```

Fitting: the CSV outcome is expressed strength, so fit in two passes. Fit `E`'s three constants on windows whose energy state changed inside the 14 weeks (deficit → surplus and the reverse). Fit `a, b, c, c_cut, d` on windows with a steady energy state, after dividing the observed gain by the modelled change in `E`. Marginal rates from the log that the combined model must roughly reproduce (per week, lb on the expressed total):

| dial | value | observed |
|---|---|---|
| near-max sets `N` | ≤2 / 2–4 / 4–6 / 6–10 / 10+ | −2.9 / −0.3 / +1.2 / +2.4 / +2.5 |
| working sets `W` | <10 / 10–15 / 15–20 / 20–25 / 30+ | −0.5 / −0.2 / +0.5 / +1.1 / +2.8 |
| weight rate `r` | < −0.5 / −0.5..−0.2 / −0.2..0.1 / 0.1..0.35 / 0.35..0.6 / > 0.6 | −1.4 / −1.0 / −0.2 / +0.7 / +1.7 / +1.1 |
| cut cost | per lb of bodyweight lost, 2024 and 2025–26 | −3 to −4 lb of total |

The bins are marginal and confounded (near-max sets and sessions rise together), so fit jointly. Windows overlap (step 2 weeks), so treat the effective sample as ~45, not 174; don't over-parameterise.

Per-lift split: distribute `ΔS` across squat, bench, deadlift in proportion to each lift's share of `N`, times a frequency factor (2 heavy exposures a week = 1.0, 1 = 0.6; two bench days averaged +35 per 14 weeks in the log, one day +7). Press follows bench's factor at 0.55× the bench change.

The post-cut rebound is now emergent: `dep` decays to 0 over the first three weeks of the surplus and `E` recovers ~6%, so no separate rebound term is needed. Use RPE-based implied maxes (working-max controller, §1.2 there) as the preferred measure of `S_obs` whenever a reading exists, and the Epley index only as the fallback; the index under-reads in cuts.

## 3. Climbing

```
C_pot   = C_skill + 0.06 · (168 − BW)              # ±0.06 V per lb around 168; cap ±1.0
ΔC_skill = e · k_c · (K/3)^0.7 · (1 + 0.3·H) · (1 + 0.5·K_lim/max(K,1))   − 0.01·[K < 1]
k_c     = 0.013 per week
C       → C_pot with a 4-week lag
```

Anchors: with 3 sessions, a hangboard and one limit session a week, this gives about +1 V grade per year, which is the fast end for someone at V6–V7 with a 2-year base. 2 unstructured sessions → about +0.5/yr, which is my 2024–25 (V6 plateau for seven quarters at 171–182). 1 session → hold. 0 → −0.3/yr. The bodyweight term is from the log's single clean contrast: V6 ceiling at 171–182, V7 at 167.

## 4. VO2 max

Track absolute capacity and divide by bodyweight; the bulk lowers the score on its own.

```
Vabs      = VO2 · BW_kg / 1000                     # L/min; 52 at 163 lb → 3.85
ceiling   = 58                                      # assumed; expose as a parameter
Z_eff     = Z + 0.25 · (Z2 / 60)
ΔVabs     = e · k_v · Z_eff · (1 − VO2/ceiling)  − δ · Vabs · [Z_eff < 0.5]
VO2_next  = Vabs_next / BW_kg_next · (1 + 0.02 · ΔLM/LM)   # gained lean mass consumes a little oxygen
```

Set `k_v`, `δ` to reproduce: 1 session a week holds absolute capacity (the score then falls only through bodyweight); 2 a week gain about +1 point per 8 weeks at 52 and slow toward the ceiling; 3 a week about +1.5; 0 a week loses about 1 point per 8 weeks (climbing and lifting protect some of it). Consequence the view must surface: going from 154 to 169 is +9.7% bodyweight, so at 1 session a week the score drops from ~53 (at 154) to ~49 by Dec 2027; holding 52 through the bulk needs 2 sessions a week from block 3 on.

## 5. Body composition

```
surplus (r > 0):  λ = clip(0.65 − 0.5·max(0, r − 0.3), 0.25, 0.70) · pf(P) · tf(W)
                  ΔLM = λ·r,  ΔFM = (1 − λ)·r
deficit (r < 0):  μ = 0.15 if (|r| ≤ 0.5 and W ≥ 15 and bench_days ≥ 2) else 0.30
                  ΔLM = μ·r,  ΔFM = (1 − μ)·r
pf(P) = 1.0 at P ≥ 0.8 g/lb, 0.85 at 0.6, 0.7 at 0.5
tf(W) = 1.0 at W ≥ 20, 0.8 at 15, 0.6 at 10
BF%   = FM / BW
```

Anchors: 13% at 154 with LM ≈ 134; gaining 15 lb at 0.2–0.4/week with protein at 0.9 g/lb lands at 15–16% (55–65% lean); 17 lb at 0.9/week (2025) should come out worse, around 40% lean.

## 6. Calisthenics

```
ΔM = e · 0.05 · (Q − 1)  − 0.03·[Q = 0]  − 0.02 · ΔBW      # reps of capacity per week
```

One session a week holds; zero decays a rep in about 8 weeks; each pound of bodyweight costs a little. Pull strength from climbing and weighted pull-ups adds `+0.01·[K ≥ 2]`.

## 7. Injury (optional module, Monte Carlo)

Weekly hazard `h = 0.004 + 0.008·H + 0.008·K_lim + 0.02·max(0, F − 0.5) + 0.02·[heavy squat or deadlift on a climbing day]`. On an event: 3 weeks with `N = 0` on the affected lifts and `K_lim = 0`; finger/elbow events also set `H = 0` for 4 weeks. Run 200 paths and report the median and the 20th percentile of each output, so the view shows "V8 by June: 45%" rather than a single line.

## 8. Simulation harness

- Time step: one week. Horizon: to Dec 5 2027 by default, or any date.
- Inputs: the block calendar from the program (dates, emphasis, week types), and a dial set per block. A scenario = the baseline dial sets with overrides ("blocks 2–7: K = 4"; "all blocks: Z = 2"; "block 5: N = 8").
- Every week: compute `L`, `F`, `e`; apply §2–§6; apply the light-week and test-week rules from the program (light week: stimulus × 0.4, `F` halved; test week: `N` counts the four singles).
- Outputs per week and at horizon: `S` and per lift, `C` (and P(V8 sent by date) from the Monte Carlo), `VO2`, `BW`, `BF%`, `F`, weeks over budget, injury-weeks.
- Always report the baseline plan next to the scenario, and flag any week where `L > L_cap` in red, because the model's confidence collapses there (it is extrapolating into the pattern that failed).

### Scenarios to ship as presets

| preset | change from baseline | what it's for |
|---|---|---|
| Climb more | `K = 4` all year, `H = 1` all year | see the budget bite lifting |
| Lift more | lifting emphasis in every block, `K = 2` | the pure-strength year |
| Cardio up | `Z = 2` from block 3 | hold VO2 at 52 through the bulk |
| Cardio off | `Z = 0` | see the score fall to ~46 |
| Drop calisthenics | `Q = 0` | see `M` decay |
| Fast bulk | `r = 0.9` blocks 2–5 | the 2025 rerun: more fat, no more strength, over budget once climbing starts |
| Stay light | hold 160 from block 4 | climbing-first year |

## 9. Calibration and acceptance

1. Fit `a, b, c, d` in §2 on `calibration_windows.csv` (ridge or grid search), holding the functional form. Report the fit on the after-window gain too. Expect R² around 0.25–0.30; this is a noisy target.
2. Replay the log through the model with the actual weekly dials from `calibration_weekly.csv` (climbing known from Aug 2024; before that assume `K = 0`) and check the model's `S_obs` tracks the estimated total within ±40 lb through: Bulk C (rise ~30 then fade as `F` builds at 181+), the 2024 cut (fall ~50, mostly `E`), Dec 2024–Mar 2025 (+55 with `E` recovering and capacity rising), Bulk D weeks 11–19 (stall as `L` exceeds `L_cap`), the 2025–26 cut (fall ~75 on the index, of which capacity should account for no more than ~30).
3. With the baseline dials and calendar, the horizon output should be near: expressed total ~1040 (990–1080) at 169 lb, of which ~60 is `E` recovering from the cut and the rest is capacity built in the three lifting blocks; BW 169; BF 15–16%; `C` ≈ 7.3 with P(V8) ≈ 0.45; VO2 ≈ 49 at `Z = 1` and ≈ 52 at `Z = 2`. If it isn't, the priors are off; say which term.
4. The "Climb more" preset must show lifting gains falling in lifting blocks (over budget), and the "Fast bulk" preset must show more fat, no extra strength above `r = 0.5`, and `F` climbing once `K` rises.
5. Every parameter is exposed in the view with its prior and its source (log / literature / assumption), so I can move any of them and see the horizon change.

Everything in §3–§6 is priors, not fits; the log only calibrates strength and the budget. Label it that way in the UI.
