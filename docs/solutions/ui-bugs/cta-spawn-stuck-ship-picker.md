---
title: Start Game click hangs on ship-picker when CTA spawn can't find open water
category: ui-bugs
component: client/src/ui/game_ui.rs
problem_type: client_ui_lockup_on_silent_server_failure
symptoms:
  - "Click Start Game in CTA mode → ship-picker overlay never dismisses"
  - "Background game is visible and running; minimap shows ships; can't interact"
  - "Server log shows flow fields built (match started) but player stays on picker"
  - "Server log: 'failed to find enough space to spawn' or 'couldn't spawn X'"
severity: high
resolution: fixed
fixed_in: warships/ba971bc
affected_platforms: [all]
related_files:
  - client/src/ui/game_ui.rs
  - server/src/world_spawn.rs
  - server/src/match_state.rs
tags: [cta, spawn, ship-picker, ui-lockup, retry, gloo-timers]
---

# Start Game click hangs on ship-picker when CTA spawn fails

Click Start Game in Capture the Area → the ship-picker overlay stays
visible forever. The server received the Spawn command and started the
match (log: `flow fields built in X ms`), but the client UI never
transitions to the in-match view. Background gameplay renders
underneath the stuck overlay; the minimap shows ships moving; the
player can't do anything.

## Symptom chain

Client side, `game_ui.rs:371` gates the spawn-screen overlay on
`UiStatus::Spawning`. The overlay only dismisses when the server flips
the player's status to `Alive`. If the server never does, the overlay
sticks.

Server side, `world_spawn.rs` has a retry loop that picks positions
inside the spawn radius around the player's assigned pentagon slot
(position computed in `match_state.rs`). In CTA, the pentagon slot is
often close to (or inside) a base-adjacent area. Two failure modes:

1. **Slot clipping a bot.** By the time a human clicks Start Game, the
   9 bots are already occupying their slots. The human's assigned slot
   position is sometimes < 1 ship-length from a bot's position, which
   `can_spawn` rejects (collision threshold).

2. **Slot clipping a surviving sparsen island.** After the terrain
   sparsen pass (30% of land survives inside the CTA corridor), a
   pentagon slot can land on one of the surviving islands. The retry
   loop's wiggle radius is 200m but if multiple islands + bots are
   clustered near the slot, no valid point exists.

In both cases the server logs the failure (`[WARN server::world_spawn]
couldn't spawn X` or `Command resulted in failed to find enough space
to spawn`) and returns without transitioning the player to Alive. The
client has no signal that anything went wrong.

## Root cause

Asymmetric signaling between server and client. The server knows the
spawn failed; the client only knows the status stayed Spawning. There's
no "spawn failed" message in the wire protocol, and even if there
were, adding one would require server-side changes to the mk48
descendant.

## Fix

Client-side retry. When `UiStatus == Spawning` AND the ship-picker is
showing, re-emit the Spawn command every 2 seconds until the status
transitions away. Uses `gloo_timers::callback::Interval` inside a
`use_effect_with` hook — the interval is dropped automatically when
the deps change (status → Playing → effect cleanup → interval
dropped).

```rust
// client/src/ui/game_ui.rs:161 (after on_start_from_picker)
{
    let play_cb = on_play.clone();
    let spawning = matches!(status, UiStatus::Spawning);
    let at_picker = *title_step == TitleStep::ShipSelect;
    use_effect_with((spawning, at_picker), move |(spawning, at_picker)| {
        if !(*spawning && *at_picker) {
            return Box::new(|| ()) as Box<dyn FnOnce()>;
        }
        let interval = gloo_timers::callback::Interval::new(2000, move || {
            play_cb.emit(default_alias());
        });
        Box::new(move || drop(interval)) as Box<dyn FnOnce()>
    });
}
```

Same pattern as `cta_respawn_overlay.rs:12,37,40` (existing interval
usage in the codebase).

## Prevention

**Server-side options we did NOT take (listed for future reference):**

- Evict nearest bot on human CTA spawn to free the slot. Bigger
  change, crosses the human-vs-bot priority boundary.
- Widen `spawn_radius` further (already bumped 30 → 200). Pushing to
  400+ risks spawning humans off-base, outside the pentagon geometry.
- Add a `SpawnFailed` wire message. Mk48's descent doesn't have it;
  retrofitting is invasive.

**Client-side retry was the minimum-viable fix** because:
- Fails closed: user sees retry-in-progress instead of a dead UI.
- No server changes needed.
- Handles any server-side spawn failure, not just the specific ones we
  identified today.

## What to check if this happens again

1. Server log for the match at the time of the stuck UI:
   ```
   grep -E "couldn't spawn|failed to find enough space" server.log
   ```
2. Current spawn_radius in `server/src/world_inbound.rs` (was 30 →
   200; further increases are possible).
3. If retry is looping forever (more than ~5 attempts), something
   more fundamental is broken: check `match_state.rs` pentagon-slot
   geometry vs. current `ArenaLayout::DEFAULT` base positions. If the
   slot is OUTSIDE the arena radius, no amount of wiggle will find
   water.

## Related

- `decisions.md` entry: "2026-04-15: Flow-field pathfinding + state-machine bot AI + arena expand"
- `plans/cta-arena-expand-and-sparsen.md` — the arena expansion that
  introduced the 30% surviving-island risk
- `server/src/world_inbound.rs` — where `spawn_radius` is configured
- `server/src/world_spawn.rs` — the retry loop itself
