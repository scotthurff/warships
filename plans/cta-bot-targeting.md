# feat: CTA bots — smarter offense/defense targeting

> **Status:** Planned — 2026-04-15
> **Depends on:** `plans/cta-carve-arena.md` (ships and stabilizes first)
> **Scope:** ~40 LOC in `server/src/bot.rs`, one PR
> **Motivation:** Playtest: "make bots more aggressive going after
> enemy territory, but have some awareness of their own territory
> and defend if they're close enough"

## Problem

Today's CTA bot targeting (`server/src/bot.rs:479-501`) is a
team-wide binary:

```rust
let target = if defense_progress_ms > 5_000 {
    own_base
} else {
    enemy_base
};
```

Two failure modes:

1. **Everyone panic-retreats.** One enemy inside our base triggers
   a 5-second countdown; when it flips, every teammate pivots
   defensive, including the bot across the arena that can't arrive
   before the 30-second capture completes.
2. **Nobody defends proactively.** A bot parked next to the base
   stays on offense until the 5s threshold, even while an enemy
   is actively inside.

## Fix

**Per-bot decision: defend if I'm the closest teammate to the base
AND there's an enemy near it. Otherwise push.**

Boolean, not threshold-based:

```rust
fn pick_target(
    own_base: Vec2,
    enemy_base: Vec2,
    i_am_closest_to_own_base: bool,
    enemy_is_near_own_base: bool,
    capture_near_completion: bool,   // defense_progress_ms > 20_000
) -> Vec2 {
    if (i_am_closest_to_own_base && enemy_is_near_own_base)
        || capture_near_completion
    {
        own_base
    } else {
        enemy_base
    }
}
```

Three booleans, two branches, one pure function. Unit-testable in
isolation — `pick_target` has zero world-state dependencies, so the
test harness is five inputs / one output.

## Computing the inputs

In the existing per-bot closure at `server/src/bot.rs:461-506`
(already borrows `server.player`, `server.match_state`, and `server.world`
via `get_player_complete`), before calling `bot_state.cta_movement_target =`:

```rust
const ENEMY_PROXIMITY: f32 = ArenaLayout::DEFAULT.base_radius * 1.2;
const CAPTURE_EMERGENCY_MS: u128 = 20_000;  // 2/3 of full capture

let my_pos = match player_tuple.borrow_player().status {
    Status::Alive { entity_index, .. } =>
        server.world.entities[entity_index].transform.position,
    _ => return, // no target needed — we're dead
};

// Who's the closest teammate to our own base, including me?
// We scan once per tick per bot, which is O(teammates) per bot —
// ~5 per tick per bot × 9 bots × 20 Hz = 900 comparisons/sec.
let my_dist_sq = (my_pos - own_base).length_squared();
let i_am_closest = server.player.iter_borrow()
    .filter(|p| p.match_team == Some(my_team))
    .filter_map(|p| match p.status {
        Status::Alive { entity_index, .. } =>
            Some(server.world.entities[entity_index].transform.position),
        _ => None,
    })
    .all(|pos| (pos - own_base).length_squared() >= my_dist_sq);

// Any enemy inside my base's proximity?
let enemy_is_near = server.player.iter_borrow()
    .filter(|p| p.match_team.is_some() && p.match_team != Some(my_team))
    .filter_map(|p| match p.status {
        Status::Alive { entity_index, .. } =>
            Some(server.world.entities[entity_index].transform.position),
        _ => None,
    })
    .any(|pos| (pos - own_base).length_squared()
        < ENEMY_PROXIMITY.powi(2));

let capture_emergency =
    defense_progress_ms as u128 > CAPTURE_EMERGENCY_MS;

let target = pick_target(
    own_base, enemy_base,
    i_am_closest, enemy_is_near,
    capture_emergency,
);
```

Entity position lookup uses `server.world.entities[entity_index]`
— the exact pattern in `collect_boat_snapshots` at
`server/src/server.rs:183-209`. No invented API.

## Offense bump

One additional tune, co-located because it's cheap and targets the
same complaint ("bots feel passive on offense"):
`server/src/bot.rs:257` objective weight out-of-combat **2.5 → 3.5**
(combat weight stays `0.8`, unchanged).

This is one line + a named constant. If the playtest shows bots
overshooting targets, revert.

## Acceptance

- [ ] `pick_target` has unit tests covering all four relevant
      branches: (A) lone bot, no enemies → push; (B) closest bot +
      enemy at base → defend; (C) not-closest + enemy at base →
      push (let nearest handle it); (D) capture_emergency → defend
      regardless of distance rank.
- [ ] Manual match: human holds center. Teammate bots consistently
      push enemy base past the human, not orbit midfield.
- [ ] Manual match: human drives a red bot into the blue base.
      The **single nearest** blue teammate pivots to defend —
      not all of them.
- [ ] Manual match: stale capture progress (red bot died inside
      base mid-capture) does NOT trigger retreat until
      defense_progress crosses 20s.
- [ ] Per-difficulty feel preserved: Captain bots still easier than
      Admiral still easier than Fleet Commander.

## Test plan

**Unit tests** on `pick_target` (~8 assertions):

```rust
#[cfg(test)]
mod targeting_tests {
    // branch A: open field → push
    // branch B: closest + enemy at base → defend
    // branch C: not-closest + enemy at base → push
    // branch D: emergency override → defend
    // edge: closest + no enemy → push (don't idle at base)
    // edge: emergency + not closest → defend (all-hands)
}
```

**Manual smoke test** on prod, iPad + desktop: the four acceptance
bullets above, across one 5-minute match plus one Play Again to
verify behavior resets cleanly across matches.

**No test for the scan cost.** 900 comparisons/sec at a hot path
isn't "negligible" (Kieran's accurate pushback on the prior plan's
language) but it isn't measurable either. If frame time regresses,
profile; otherwise leave.

## Explicit non-goals

- Pathfinding. Bots still use the aggregate-force movement model.
- Coordination ("wait for teammate before attacking"). Future plan.
- Fire-control redirection. Defenders still shoot at the nearest
  enemy in sensor range, same as today. Out of scope.
- Per-bot aggression persistence across matches. The existing
  `aggression: rng.gen_range(...)` randomization per-bot at
  `server/src/bot.rs:76` stays.

## Files

| File | Change | Rough LOC |
|------|--------|-----------|
| `server/src/bot.rs` | Extract `pick_target` pure function; replace the threshold binary with the per-bot scan; bump objective weight 2.5 → 3.5 | ~40 |
| `server/src/bot.rs` (test module) | Unit tests for `pick_target` | ~25 |

One PR. Depends on `cta-carve-arena.md` being merged and verified
first so this plan's behavior changes can be observed without
confound.

## References

- `server/src/bot.rs:461-506` — current per-tick CTA-awareness
  closure where the new scans land
- `server/src/bot.rs:479-501` — current `movement_target` binary
- `server/src/bot.rs:252-260` — objective pull weight (2.5 in open)
- `server/src/bot.rs:76` — per-bot `aggression` randomization
- `server/src/server.rs:183-209` — `collect_boat_snapshots` —
  the exact pattern for reading entity positions via
  `server.world.entities[entity_index]`
- `server/src/match_state.rs:30-48` — `ArenaLayout::DEFAULT`
  (`base_radius = 250` → ENEMY_PROXIMITY ≈ 300)
- Commits: `715e93e` (bot AI overhaul), `efe6302` (CTA teammates
  friendly) — prior context
