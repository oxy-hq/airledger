# Coach chat redesign (v2 — supersedes the draft-rows delivery in
# 2026-09-11-nightly-coach-design.md)

**Date:** 2026-09-12
**Status:** Approved (user chose Mac-relay chat, kill draft rows,
pinned coach row; then "continue without confirmation").

## Why v2

User feedback on v1: coach-written draft rows overload the logging
mechanism and read as logged entries. The coach should be a
first-class, *separate* surface — a chat — with a home-screen presence,
sitting above the logs because it ingests all of them.

## Decisions

1. **Mac-relay chat.** Coach messages and user replies live in a synced
   `coach_chat` table (rides the existing ledger↔Sheets pipe) but are
   RENDERED as chat, never as log rows. A Mac-side relay watches for
   user messages and replies via `claude -p` on the Max plan.
2. **No coach-written rows in logs.** The briefing describes the
   session in text (naming templates); the user applies templates
   himself. Timeline draft UI is reverted; yesterday's 24 auto-drafted
   rows are deleted; the `coach_log` view is retired (its one briefing
   seeds the chat).
3. **Pinned Coach row** at the top of the home screen: bot icon,
   tinted distinctly from log tiles, preview of the latest coach
   message, unread accent until opened. Tap → coach chat screen.

## Components

### coach_chat view (fitness repo; replaces coach_log)

Dimensions: `id` (uuid), `date` (date, message day), `ts` (string, ISO
datetime, ordering), `role` (`coach`|`user`), `kind`
(`briefing`|`reply`|`user`), `text` (string). Input overlay exists so
the view syncs (gsheets entry view), but the app hides it from the
normal tile list and renders it only through the coach surfaces.

### App (airledger-archive)

- **Revert** timeline draft treatment (restore
  `lib/ui/timeline_screen.dart` to its pre-draft state; the
  noon-planning-target tool fixes stay). Planned-vs-logged distinction
  is again solely PlanStore's planned section, which already renders
  separately — the confusing case (coach rows) no longer exists.
- **Home screen:** view named `coach_chat` is excluded from the tile
  list. A pinned Coach row renders first: bot icon, tinted container,
  subtitle = first line of the newest coach message + relative time;
  unread state (newest coach `ts` > ledger-meta
  `coach_chat_last_read_ts`, meta is device-local) shows an accent
  dot/tint. Tap opens the chat screen. Hidden when the view is absent.
- **Coach chat screen** (`lib/ui/coach_chat_screen.dart`): renders
  coach_chat rows sorted by `ts` as bubbles (coach left, user right);
  composer appends a `role=user, kind=user` row via the normal
  repository create and triggers a manual sync; screen refreshes on
  sync completion + a periodic poll while open (coach replies arrive
  via the relay within a couple of minutes); marks read by writing
  `coach_chat_last_read_ts`. Shows a subtle "coach will reply in a
  minute or two" hint when the last message is the user's.

### Mac side (tools + launchd)

- `tool/coach_dump.dart` — unchanged (context source).
- `tool/coach_apply.dart` — DELETED (draft writing retired).
- `tool/coach_msg.dart` — new: subcommands
  `post` (append a coach_chat row; args --role --kind, text on stdin),
  `pending` (exit 0 + print chat history when the newest message is a
  user message needing a reply; exit 3 when nothing pending),
  `briefing-exists --date D` (exit 0/3) for nightly idempotence.
- `coach/PROMPT.md` v2 (fitness repo) — two modes, both PLAIN TEXT
  output (no JSON): BRIEFING (morning message: today's session per
  routine/metrics/notes, why, flags, named template) and REPLY
  (conversational answer given chat history + ledger context).
- `tool/coach_nightly.sh` — rewritten: idempotence check → dump →
  claude (BRIEFING) → `coach_msg post --role coach --kind briefing`.
- `tool/coach_relay.sh` — new: `coach_msg pending` → if pending, dump
  + history → claude (REPLY) → post `--kind reply`.
- launchd: `com.robertyi.airledger-coach` 23:30 nightly (briefing);
  `com.robertyi.airledger-coach-relay` StartInterval 120 (replies;
  no-op when nothing pending — one Sheets read per tick).

### Cleanup

Delete the 24 draft strength rows dated 2026-09-12 (blank start_time,
appended 2026-09-11 22:37) and the `coach_log` tab; remove
coach_log.view/input.yml; seed coach_chat with yesterday's briefing as
the first coach message (kind briefing, date 2026-09-12).

## Error handling

- Relay claude failure → logged, message stays pending, next tick
  retries. Post failures never drop text (logged to file).
- Chat screen tolerates empty/missing view (row hidden, screen guard).
- Unread meta missing → treat all as read except a newer coach ts.

## Testing

- Tools: `coach_msg` post/pending/briefing-exists round-trip against
  the live sheet (a test post then real seed).
- App: `flutter analyze` + suite (7 known pre-existing failures), build.
- E2E: seed briefing visible in chat + coach row; post a user message
  via the phone-equivalent path (tool), run relay once, confirm a
  coach reply lands; verify on-device after install.
