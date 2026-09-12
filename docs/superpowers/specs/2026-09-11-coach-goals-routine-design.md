# Coach goals + routine codification — Design

**Date:** 2026-09-11
**Status:** Approved (user: "yeah good to go")
**Parent effort:** AI daily coach, sub-project 2 of 4.

## Goal

Codify the user's objectives and weekly training structure as durable,
Claude-editable documents that (a) answer "what should I be doing" in
rule form, (b) link each rule to the Air Ledger template that implements
it, and (c) serve verbatim as prompt context for the nightly coach.

## Design

New `coach/` directory in `~/repos/airledger-fitness` (docs only, no code):

- `coach/README.md` — the contract: Claude edits these files in
  conversation and pushes; template references into `views/*.template.yml`
  are load-bearing (they ARE the structured plans); files are read
  verbatim as coach prompt context.
- `coach/goals.md` — active phase (cut, as of 2026-09-11) + per-domain
  objective table: VO2max progress, climbing progress, heavy lifts
  maintain, muscle-up skill progress, nutrition deficit adherence,
  everything else unprioritized.
- `coach/routine.md` — cut-phase weekly rules, each linked to its
  template: 1× 4×4 (`cardio.treadmill_4x4`); 2× climbing
  (`strength.climbing_prep` for gym prep); one heavy lower per week
  alternating squat/deadlift with the off-lift light (+ reentry
  variants); 1× muscle-up day (`strength.cut_muscle_up`); bench+OHP as
  separate or combined day (`strength.cut_press_heavy`,
  `strength.cut_press_deload`). Plus coach scheduling guidance
  (spacing, deriving week A/B from the ledger, daily notes outrank the
  rotation).

## Review model

The committed markdown files are themselves the review surface — the
feature IS user/Claude co-editing of these docs. Redlines happen as
normal edits in future sessions.
