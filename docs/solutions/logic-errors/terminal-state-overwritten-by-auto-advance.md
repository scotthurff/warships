---
title: Terminal state overwritten by tick-loop auto-advance
category: logic-errors
component: server/match-state
problem_type: state_machine_bug
symptoms:
  - "Match-end interstitial never renders"
  - "Client skips directly from Playing → Countdown, bypassing Ended"
  - "UI tied to a terminal phase flashes for one frame or not at all"
severity: high
resolution: fixed
fixed_in: db75e18
related_files:
  - server/src/server.rs
  - client/src/ui/match_end_overlay.rs
  - server/src/match_state.rs
tags: [state-machine, fsm, tick-loop, rust, ui]
---

# Terminal state overwritten by tick-loop auto-advance

## Symptom

After a Capture-the-Area match ended, the full-screen results interstitial
(`MatchEndOverlay`) never appeared. The client went straight from `Playing`
back into the ship picker / Countdown.

Observable:
- The match clock hit `0:00`, team scores finalized, but the screen flicked
  from combat → ship picker with no "BLUE WINS" / "RED WINS" / stats table.
- Play Again and Quit buttons had nothing to dismiss — the UI was already gone.

## Root cause

The server tick loop (runs every 100 ms) had a guard that auto-started a new
match when the current phase was *either* `Waiting` *or* `Ended`:

```rust
// server/src/server.rs — BUG
if matches!(self.match_state.phase, MatchPhase::Waiting | MatchPhase::Ended { .. }) {
    self.match_state.start_match();   // → phase = Countdown
    self.assign_match_teams();
    self.clear_statics();
}
```

Same pattern existed in the `Spawn` command handler.

This was written as "auto-start whenever nothing is in progress," but it
collapsed the FSM's *initial* state and its *terminal* state into the same
branch. The moment `MatchState::tick()` transitioned `Playing → Ended`, the
very next tick (100 ms later) transitioned `Ended → Countdown`. The client's
`MatchUpdate` stream never carried `phase = Ended` for long enough to render.

In short: a terminal state that only persists for 100 ms isn't terminal.

## Fix

Restrict auto-advance to `Waiting` only. `Ended` persists until the human
takes a deliberate action (`PlayAgain` → `match_state.reset()` → `Countdown`,
or `QuitToTitle`).

```rust
// server/src/server.rs — FIXED
if matches!(self.match_state.phase, MatchPhase::Waiting) {
    self.match_state.start_match();
    self.assign_match_teams();
    self.clear_statics();
} else {
    // Mid-match: new bots join whichever side has fewer ships.
    self.assign_late_joiners();
}
```

Applied to both sites (tick loop and `Spawn` handler). Commit `db75e18`.

## Why the bug was hard to spot

1. The `matches!(phase, Waiting | Ended)` pattern reads as correct intent —
   "start a match if nothing is active." But `Ended` carries the **winner**
   field and is load-bearing for client UI. It isn't "nothing active."
2. Single-tick state transitions don't show up in logs unless you're dumping
   every phase change. The server was technically *correct* on every tick;
   each individual decision was consistent. Only the 100 ms lifetime of
   `Ended` was the problem.
3. The client correctly received `phase = Ended` — just for a single frame,
   often during the render interval of the previous frame, so Yew had no
   chance to render `MatchEndOverlay` before it saw `Countdown` again.

## Prevention

- **Never collapse "initial" and "terminal" states into a single auto-
  advance branch.** If your tick handler has `matches!(state, A | B)`, ask:
  is `B` a terminal state that UI reads? If so, exclude it.
- **Terminal UI states need a human-gated transition out.** `Ended` should
  only be exited by `PlayAgain` or `QuitToTitle` — explicit user input, not
  a timer. Auto-advancing terminal phases is a symptom of state-machine
  smell: either the state is genuinely terminal (don't auto-advance) or it's
  a transient status (give it a different name like `Cooldown`).
- **When adding UI that reads a server-authoritative enum, list every phase
  the UI can observe and grep the server for each.** Every write site to
  that field should be intentional for every variant.
- **Rule of thumb:** if a phase exists so the client can render something,
  the server must leave it alone long enough to be rendered. The minimum
  duration of any observable phase should exceed one tick period by at
  least an order of magnitude — or it should be gated on client
  acknowledgement rather than wall-clock time.

## Related symptoms to watch for

- "Toast notification flickers but never renders" — often the server
  clears the toast state before the next client poll.
- "Modal auto-dismisses after an action" — same FSM collapse, different
  domain.
- "Retry button never appears on error" — error state overwritten by
  re-request on next tick.

If you see a UI that "should appear but doesn't," and the backing state
comes from a server tick loop, grep for `matches!(…, Foo | Bar)` patterns
and verify that no branch in the `|` is a state the UI depends on seeing.
