# Training simulator v2.1 — fit / replay / horizon report (2026-09-26)

Companion to `2026-09-26-training-simulator-spec.md` (the REVISED spec,
§2 "capacity and expression are two different things"). Supersedes the
wave-1 (pre-revision, one-layer §2) results, which are summarized in §0
for reconciliation. Tools live in the ledger repo:
`tool/sim2_model.dart` (shared core), `tool/sim2_fit.dart` (§9.1),
`tool/sim2_replay.dart` (§9.2), `tool/sim2_horizon.dart` (§9.3/§9.4).
Ledger commit: `39113c0`.

## 0. Wave-1 baseline (superseded)

One-layer §2 with an explicit post-cut rebound term. Ridge fit a=4.06
b=0.51 c=1.59 d=1.54 (R² 0.331 train / 0.277 validate); replay 6/8 with
two documented fails — Bulk-C peak −47 (gain-columns vs level-columns
basis disagreement in the calibration CSV itself) and Dec-2024 trough
+59 (the model had no expression dip); horizon old-basis 971
(§9.3-of-v2.0 band 940–1020). The Dec-2024 trough fail is precisely the
phenomenon the revision formalizes; v2.1 fixes it (err +4). The Bulk-C
level anomaly persists in v2.1 (−47) — it is in the data, not the model.

## 1. What changed in v2.1 (revised §2)

- Two layers: `S_obs = S_cap · E`. Capacity moves slowly (training,
  food, time); expression `E = 1 − eDep·dep + eBw·(BW−165) − eRust·rust
  − 0.03·F` with dep ramping 0→1 over 3 weeks in a deficit / decaying
  over 3 weeks in a surplus, rust ramping over 4 weeks at N<2 / clearing
  over 2 weeks at N≥4.
- The v2.0 rebound term is REMOVED — the post-cut snap-back is emergent
  from dep decay (verified: Dec-24→Mar-25 segment reproduces +55).
- Food term split: `c` (surplus, clip 0..+0.5) vs `c_cut` (deficit),
  priors 1.5 / 1.0; a≈2.5, d≈0.6.
- S_obs basis: RPE-implied readings preferred where they exist; the
  Epley index is the fallback. BOTH calibration CSVs are index-basis, so
  the fit and replay run on the index; the horizon seeds capacity from
  the Sep-2026 RPE readings (917) and reports true expressed strength.

### 1.1 Epley-index measurement model (new, [fit]/[assume])

The window fit initially attenuated ALL of E to zero. Diagnosis: the
app's index is built from whatever sets happened, so it re-reads true
expressed strength only when near-max attempts occur. Log evidence: the
Dec-2024→Mar-2025 snap-back shows up on the index within weeks
(N≈13.7/wk then), while the 2025-26 cut's attempt-sparse onset shows
only −12 over 14 weeks as E crashed, the fall arriving with the N≈8–13
attempt burst of Feb–Mar 2026. Model: `index(t) = S_cap(t) ·
gatedEma(E, N; kAttempt, kIdle)` — gate N≥2, fitted kAttempt=0.7,
kIdle=0.1. Used ONLY against the index (fit, replay); horizon output
carries no smoothing. Two corollaries worth keeping:

- today's 878 (index) vs 917 (RPE) gap ≈ the attempt-sparse under-read;
  it closes as near-max attempts resume in the bulk (measurement
  catch-up, not physiology);
- the energy-state instrument for dep (hysteresis on a 4-wk MA of bw
  diffs, min-run 4, detection-lag shift 3 wk) is the noisiest part of
  the pipeline — it drives the identification limits below.

## 2. §9.1 two-pass fit

Protocol: per measurement-model cell (kAttempt × kIdle × eDep), two
alternating sweeps of pass 1 (E on the 139 energy-state-flip windows,
grid + ridge toward priors, data term scaled by the residual variance
σ̂²≈6.5 (lb/wk)² and by the effective-n weight 45/174) and pass 2
(capacity on the 31 steady windows, linear ridge on the capacity-basis
target `ΔS_cap = (S0+gain)/E_idx_end − S0/E_idx_start`, i.e. observed
gains divided through the modelled E path).

**Identification finding (the important caveat):** the window-difference
likelihood is FLAT in eDep — any value in [0, 0.06] sits within ~2σ.
Its instrument (bw-trend energy-state detection) is timing-noisy, and a
1–2-week mistiming of a sharp E swing wrecks 14-week DIFFERENCES while
LEVEL tracking stays tight. So eDep and the measurement cell are
selected by the §9.2 replay checkpoints (level-based, the spec's own
acceptance harness); eBw/eRust ride their clean instruments (logged bw,
logged near-max sets) inside the ridge. The frontier, explicitly:
window R² is maximized (~0.17) with E≈0 and replay then fails the cut
checkpoints; at the replay-selected E, R² on 14-wk differences goes to
−0.66 while replay passes 4/5 with median level error 31 lb. The spec's
E anchors say the second point is the right one; both are reported.

### Coefficients (shipped = `Sim2Params.fitted()`)

| pass | param | prior | fitted | note |
|---|---|---|---|---|
| 1 (E) | eDep | 0.06 | **0.040** | replay-selected (window likelihood flat) |
| 1 (E) | eBw | 0.0025 | **0.0020** | per lb around 165; per-lift split ×1.4/1.0/0.6 (bench/squat/deadlift) |
| 1 (E) | eRust | 0.04 | **0.0075** | mostly absorbed by the attempt-gated index model |
| 1 (E) | eF | 0.03 | 0.03 | held [assume] |
| meas. | kAttempt | — | **0.7** | index convergence rate, attempt weeks |
| meas. | kIdle | — | **0.1** | attempt-sparse weeks |
| 2 (cap) | a | 2.5 | **2.49** | near-max, saturating |
| 2 (cap) | b | 0.4 | **0.39** | working volume per 10 sets about 20 |
| 2 (cap) | c | 1.5 | **1.44** | surplus food slope |
| 2 (cap) | c_cut | 1.0 | **0.92** | deficit capacity cost |
| 2 (cap) | d | 0.6 | **0.59** | drift |

Implied cut cost ≈ c_cut (0.92 lb capacity per lb lost at 1 lb/wk) +
expression leverage (eBw·S ≈ 2 lb/lb) ≈ **−3 lb of expressed total per
lb of bodyweight lost** — inside the spec's −3..−4 anchor.

R² (expressed 14-wk gains): fitted −0.66 train / −0.53 validate at the
shipped point; +0.17/+0.15 at the E→0 point of the frontier. The spec
expected 0.25–0.30 — the term to name is the E layer itself: W1's 0.33
was achieved by absorbing cut costs into capacity (a=4.06, d=1.54),
which is exactly what the revision forbids. Level-based tracking
(replay, 68 obs points over 2.7 y): median |err| 31, max 70.

### §2 marginal bins (expressed, lb/wk; fit-tool output)

| dial | bin | spec-obs | data-obs | model |
|---|---|---|---|---|
| N | ≤2 / 2-4 / 4-6 / 6-10 / 10+ | −2.9 / −0.3 / 1.2 / 2.4 / 2.5 | −3.0 / −0.4 / 1.0 / 2.4 / 2.5 | −0.3 / −0.4 / 0.9 / 0.9 / 4.0 |
| W | <10 / 10-15 / 15-20 / 20-25 / 30+ | −0.5 / −0.2 / 0.5 / 1.1 / 2.8 | −0.7 / −0.1 / 0.9 / 1.1 / 2.8 | −0.6 / −0.1 / 1.4 / 1.5 / 2.7 |
| r | <−.5 / −.5..−.2 / −.2...1 / .1...35 / .35...6 / >.6 | −1.4 / −1.0 / −0.2 / 0.7 / 1.7 / 1.1 | −1.4 / −1.0 / −0.4 / 0.7 / 1.7 / 1.1 | −1.8 / −0.2 / 0.2 / 0.5 / 1.7 / 1.1 |

Mid-bins reproduce; the extreme-N bins (≤2, 10+) are distorted by E
swings coinciding with attempt density — a known cost of the shipped
point on the frontier (the E→0 point matches those bins at −1.4/1.7).
§1 budget anchors: under / under / OVER, unchanged from W1.

## 3. §9.2 replay — five checkpoints, E vs capacity splits

2024-01-01 anchor (index 973 → S_cap 953). Gate ±40 lb on the model
index at the segment end. Split is the exact midpoint decomposition
`Δ = Ē·ΔS_cap + S̄_cap·ΔE_idx`.

| checkpoint | date | model | obs | err | gate | segment Δ model (obs) | capacity | expression |
|---|---|---|---|---|---|---|---|---|
| 1. Bulk C rise ~30 then fade | 2024-06-17 | 989 | 1036 | −47 | FAIL† | +24 (+28) | +8 | +16 |
| 2. 2024 cut fall ~50, mostly E | 2024-12-30 | 920 | 916 | +4 | PASS | −28 (−73) | −3 | **−25 (89% E)** |
| 3. Dec24–Mar25 +55, E + capacity | 2025-03-24 | 975 | 1010 | −35 | PASS | **+55** (+94) | +9 | **+46** |
| 4. Bulk D wk11–19 stall | 2025-06-16 | 984 | 987 | −2 | PASS | +9 (−23) | +6 | +4 |
| 5. 2025-26 cut fall ~75, cap ≤ ~30 | 2026-07-27 | 919 | 904 | +15 | PASS | −37 (−74) | **−6 (16%)** | −31 |

† Same −47 as W1: the CSV's own gain columns say Bulk C gained ~0 while
its level columns rose +63 from January; the model (fit on gains)
reproduces the spec's "rise ~30" in shape (+24 segment, true-E +47) but
cannot reach the level spike. Documented data-basis anomaly, carried
from W1.

The decomposition delivers the revision's claims: the 2024 cut fall is
89% expression; Dec-24→Mar-25 is +46 E snapping back + 9 capacity —
model +55 is the spec's stated anchor exactly (the raw column says +94);
the 2025-26 cut's capacity share is 16%, under the ~30/75 ceiling.
W1's Dec-2024 trough failure (+59) is resolved (+4). The 2023-04-10
anchor remains documented as out-of-form (returning-lifter regain the
model has no term for; every checkpoint shifts ≈ −50 to −110).

## 4. §9.3 horizon — Dec 5 2027 (THE ANSWER)

Start (Sep 26 2026): expressed 917 (RPE basis; index 878), capacity 968,
E 0.947 (dep=1 — the bw log says the deficit never closed through
mid-Sep; rust 0 on the RPE basis; F 0.3).

**Expressed total: 1022 (MC median; p20–p80 1013–1030; deterministic
1023) vs the spec's ~1040 [990–1080] — inside the band, ~2% under
center.** Per-lift: squat 372, bench 281, deadlift 370; press 163.
Capacity at horizon 1014, E 1.009.

**E-vs-capacity split of the gain (the payoff):** +106 from 917 =
**+45 capacity + +62 expression** — the spec's "~60 is E recovering
from the cut" lands at 62 (dep 1→0, bw leverage 163→171, F 0.3→0.1).
On the index basis (from 878) add the ~39 lb attempt-sparse measurement
catch-up: +145 total on the number the app will display.

| output | model | §9.3 expects | verdict / term to name |
|---|---|---|---|
| expressed total | 1022 [1013–1030] | ~1040 [990–1080] | in band; residual gap is the §1 budget bite (e=1 run → 1053) |
| BW | 171.2 | 169 | block-calendar r arithmetic itself sums to ~171 (also in W1) |
| BF% | 17.6 (μ=0.30) / **16.1 (μ≈0)** | 15–16 | §5's μ rule picks 0.30 at r=−0.75; the spec's own 13%-at-154 anchor implies μ≈0 → 16.1%. Both branches shipped pending the Nov DEXA |
| C | 6.97, peak C_pot 7.48 | ~7.3 | §1 budget bite: e=1 sensitivity gives C 7.23. The plan's climbing blocks run L 8.4 > 7 |
| P(V8 sent) | **0.45** (+0.4 send margin, W1 adaptation, labeled) | ~0.45 | match; P(C_pot itself ≥ 8) = 0.02 |
| VO2 | **49.6** at Z=1 / **52.0** at Z=2 | ~49 / ~52 | match (W1's maintenance-threshold VO2 form kept, labeled) |
| F / red flags | F_end 0.09; **56 over-budget weeks** | — | the cut plan itself runs L=6.9 > 6.0 every non-light week; climbing blocks L=8.4 > 7.0. Model confidence collapses in red weeks by design |
| injury-weeks | mean 2.2 (200 paths) | — | §7 module |

Block-end expressed/capacity: B0 890/961 (bw 155 — the cut trough),
B1 936/963 (reverse-diet snap-back, no rebound term needed), B3
972/981, B5 1003/1000, B7 1023/1014.

## 5. §9.4 presets (re-verified)

- **Climb more** (K=4, H=1 all year): total 968 vs 1023, lifting-block
  expressed gain +25 vs +87 baseline → falls, as required; over-budget
  weeks 62, F_end 1.06.
- **Fast bulk** (r=0.9 blocks 2–5): +14.8 lb fat vs +1.1 baseline, BW
  187, BF 23.4%, total 1011 ≤ baseline (no extra strength above
  r=+0.5), F_end 0.89 with the budget blowing once climbing starts —
  the 2025 rerun, as required.

## 6. For wave 3 (§9.5 UI)

- Every constant in `sim2_model.dart` carries a provenance tag ([fit] /
  [log] / [lit] / [assume]) — expose with prior + source per §9.5;
  §3–§6 are priors, label them so.
- The frontier caveat belongs in the UI: eDep is anchor/replay-pinned,
  not window-fitted; moving it should re-run the replay panel.
- μ branch toggle (0.30 vs ≈0) until the Nov DEXA settles §5.
- The index-vs-true distinction matters for display: the app index will
  lag the model's expressed line by the attempt gate; show both or the
  user will read the catch-up as a modeling error.
