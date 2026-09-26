# Program = simulation machine — user directive (2026-09-25, overnight build, pre-authorized)

User (verbatim, abridged where markers []): "The program tab seems
redundant with the home tab. I think the split should be that the home
tracks the progress of the program, and the program page tracks the
configuration and the phases of the program. [...] I care about three
things: 1. progress towards my goals (the output metrics - is my cut
on track, is my strength maintaining). Then key input metrics - each
week am I adhering to the plan correctly that will maximize my
results. Then, the program is my longer-term prognosis. [...] I'd want
like... a forecast of what my weight should look like as I turn the
curve to the bulk, what should be expected of my strength. Really a
simulation as I move through the phases would be great. It's a
simulation machine, really. And it should be based on my historical
progress and such. I also want the numbers used for the simulation to
adjust (and the phases) according to my weekly progress. I.e. if I've
lost a little more weight and I'm lighter, I can cut less. So really
the program is a simulation machine to plan out my goals for the
future. Once I go to the next phase, simulate what happens to my
strength. With each phase, what happens to my grades in climbing? What
happens to my lifts? Both raw strength and wilks? And when my next cut
starts, what happens, and longer term once I'm done with THAT cut,
what happens? Am I on a trajectory to continue to gain strength
indefinitely? How does this respond to different levers, like if I
gain weight faster or slower. Whatever simulation is created should
always use historical data to validate the causal impact. Within
Airlayer there is a world model, with I think a concept of drivers. It
may make sense to leverage that to implement this so it's systematic."

ADAPTATION (best guesses, user asleep):
1. IA: Home = progress (outputs + weekly drivers; unchanged). Program
   tab = configuration + phases + THE SIMULATION (forecast section
   replaces/absorbs the observed-duplicating parts; observed weight
   chart stays as the anchor the forecast extends from).
2. Model (honest, data-fit, no ML theater): weekly state
   (bw, per-lift e1RM actual+RPE-adj, wilks, climb grade p75) evolved
   by phase-conditioned response rates FIT FROM HISTORY: for each
   historical phase-window (bulks/cuts/maintains identified from bw
   trajectory + known phase dates), fit per-lift strength velocity
   (lb/wk) as fn of bw rate (the main lever) + heavy exposure; climbing
   grade velocity vs session frequency + bw. Validation = walk-forward
   backtest: fit on data before each historical phase, predict that
   phase, report MAE per output; the §-style acceptance is that
   predictions beat naive flat baselines and the 2025 bulk + current
   cut reproduce directionally. Calibration numbers + fit quality are
   FIRST-CLASS surfaced in the UI ("model: strength +0.9 lb/wk per
   +1 lb/wk bw rate, MAE 4.1 lb over 3 held-out phases").
3. Adaptive re-planning: sim starts from CURRENT observed state each
   run; phase schedule adjusts by rules — cut ends at target weight OR
   end date (whichever first; lighter-now => shorter cut, the user's
   example); bulk blocks keep dates but rate lever adjustable;
   subsequent cycle auto-generated (next cut when bw hits band top /
   hard cap, mirroring program rules) to answer the long-horizon
   "indefinitely?" question over ~3 years.
4. Levers: bulk gain rate (0.1..0.6 lb/wk), cut rate (0.5..1.6),
   climbing frequency (2/3), horizon; instant client-side re-sim.
5. Airlayer world model: INVESTIGATE ~/repos/airlayer (+ oxy refs) for
   world-model/driver concepts; if a usable driver abstraction exists,
   declare the causal graph in the semantic layer (drivers: bw_rate ->
   strength_velocity etc.) and read it from there; if not, implement a
   minimal declared-drivers YAML (airledger-fitness app/world_model.yaml)
   with the same spirit: nodes, drivers, fitted coefficients cached +
   refit nightly. Either way the causal spec is DECLARED config, code
   only evaluates it (consistent with the repo's schema-driven ethos).
6. Weekly adaptation: nightly refit/recalibrate (program_status_update
   extension) writes model params + current forecast to a tab
   (`world_model` / `forecast`) so MCP + briefings can cite them; app
   also refits on demand (pull-to-refresh).
7. Phasing: W1 research+calibration study+design doc; W2 model+sim
   core (pure Dart, TDD, walk-forward validation gate); W3 Program-tab
   forecast UI + levers + nightly wiring + MCP surfacing + install.
   Acceptance: validation report committed; forecast renders bw curve
   through 2027 program + next cycle with phase bands, strength/wilks
   + climbing tracks, levers re-sim live; sim state adapts to current
   data; all baselines hold.
