# App information architecture redesign — user directive (2026-09-22)

User (verbatim): "really should reorganize the information architecture
of this whole app. dashboard homepage, and then there's two paradigms:
data entry and integration. for each type of record (strength, weight
tracking, meals, etc.) either can be a data entry paradigm or an
integration. integrations are read-only and should have a more
read-friendly interface. data entry, of course, should follow the
standard format as given. but both should have a dashboard view
associated with them that tracks the key metrics based on goals
configured for each - e.g. powerlifting totals / e1rm and actual max
weight done for core lifts. for weight, body fat over time weight over
time. etc."

ADAPTATION DECISIONS (executing pre-authorized):
1. Presentation config is DECLARED, hot-reloadable, and engine-free:
   new `app/dashboards.yaml` in airledger-fitness (fetched via GitHub
   like coach/program.yaml, 1h cache + pull-to-refresh bust). Per
   domain: paradigm (entry|integration), source views, and a metrics
   list. NO new engine schema keys — this is presentation, not row
   semantics; trap #1 avoided entirely.
2. Home IA: dashboard synthesis stays on top; below the Coach row two
   sections replace the Ledgers expandable: LOG (entry domains: tap →
   domain screen with dashboard header + standard timeline/form) and
   CONNECTED (integration domains: read-only, read-friendly — tap →
   domain screen with dashboard header + a denser read view; no FAB,
   no forms). Week plan / Program / Apps / Integrations tiles remain.
3. Domain metric kinds (initial vocabulary, computed from existing
   services — program_metrics, weight_series, wm_store, analytics):
   stat (latest value), best (all-time), series (line chart), and
   built-ins: e1rm_reference, all_time_best_weight, pl_total (sum of
   4-lift best capped e1RM), bw_series, bf_series, kcal_series,
   protein_series, grade_pyramid, session_frequency, hr_4x4_series.
   Goals per metric (target lines) come from the same yaml; strength
   goals default from program targets.
4. Domain assignments: strength/weight/cardio/daily_notes = entry;
   climbing (kaya) + meals (macrofactor) = integration (meals rows stay
   ledger-synced; paradigm affects UI only).
5. Phasing: P1 config+loader+home IA; P2 domain dashboard framework +
   strength & weight; P3 meals/climbing/cardio dashboards +
   read-friendly record list for integration domains.
