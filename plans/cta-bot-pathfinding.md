# feat: Flow-field pathfinding for CTA bots

> **Status:** Ready to implement — 2026-04-15
> **Scope:** CTA mode only. ~180 LOC net across 3 commits in one PR.
> **Reviews:** DHH / Kieran / Simplicity (twice). Every decision below
> is the convergent outcome — no open questions remain.

## Overview

Replace the CTA bot's direct-to-base vector pull with a flow-field
sample. One flow field per goal base, built once per match from the
deterministic terrain (`SEED = 42700.0`). Bot samples the field at
its position each tick and adds the resulting direction to the
existing force-field movement vector. All other force terms
(teammate spring, enemy engagement, projectile dodge, short-range
terrain repel) stay unchanged.

## Design

```rust
// server/src/bot_pathfinder.rs — new module

pub const CELL_SIZE: f32 = 25.0;        // matches terrain SCALE
pub const GRID_SIZE: usize = 128;       // covers 1500 m arena
pub const INFLATION_CELLS: usize = 3;   // ~75 m: Iowa beam + margin

const IMPASSABLE_COST: u16 = u16::MAX;  // module-private sentinel

pub struct FlowField {
    integration: Box<[u16]>,            // GRID_SIZE² u16s, 32 KB
    flow: Box<[Vec2]>,                  // GRID_SIZE² Vec2s, 32 KB
    goal_world: Vec2,
}

impl FlowField {
    /// One Dijkstra pass from goal over 8-connected grid, straight
    /// cost 10 / diagonal 14 (√2 ≈ 1.4). Blocked cells = altitude ≥
    /// SAND_LEVEL or outside 1500 m radius; inflated by 3 cells
    /// (single Chebyshev dilation).
    pub fn build(terrain: &Terrain, goal: Vec2) -> Self { /* ~60 LOC */ }

    /// Nearest-cell lookup. Returns None if cell is blocked or pos
    /// is outside the grid; caller falls through to direct-to-goal
    /// + existing short-range terrain repel.
    pub fn sample(&self, pos: Vec2) -> Option<Vec2> { /* ~10 LOC */ }

    pub fn goal(&self) -> Vec2 { self.goal_world }
}
```

**Construction discipline:** `Vec::with_capacity(GRID_SIZE * GRID_SIZE)`
→ fill → `.into_boxed_slice()`. Never `Box::new([0u16; N])` (debug-
build stack overflow).

**Dijkstra:** hand-rolled `BinaryHeap<Reverse<(u16, usize)>>`, ~40
LOC. No crate dependency.

**Per-goal fields** on `Server`:

```rust
pub struct Server {
    // …existing
    flow_to_blue: Option<FlowField>,  // bots attacking Blue sample this
    flow_to_red: Option<FlowField>,   // bots attacking Red sample this
}
```

Built in all three CTA match-start paths (`start_match` tick branch,
`Command::Spawn` bootstrap, `handle_play_again`). Cleared in
`handle_quit_to_title` and `reset_to_waiting`.

**Integration point** — `server/src/bot.rs:252` (inner update's
objective term):

```rust
if let Some(direction) = self.cta_flow_direction {
    let weight = if closest_enemy.is_some() { 1.5 } else { 6.0 };
    movement += direction * weight;
}
```

`cta_flow_direction` is sampled in the outer update (which has
`&Server` access) and stashed on the bot. Defender bots sample
toward their own base; attackers sample toward the enemy base. At-
goal defenders get near-zero magnitude by construction — engagement
springs dominate, which is correct.

**Stuck detection** — tick-based, not wall-clock (Kieran caught
the Instant-drift issue):

```rust
pub struct Bot {
    // …existing
    cta_flow_direction: Option<Vec2>,
    cta_stuck_since_tick: Option<u32>,  // server tick count, not Instant
}
```

In the outer update, after sampling: if `boat.velocity.to_mps() <
1.0`, record the tick if not already. If 30 ticks elapsed (3 s at
10 Hz), set `cta_flow_direction = None` for the current tick so
the force field's local avoidance can push the bot out. Reset the
counter when speed recovers.

## Phase 0 — revert in-flight waypoint work (separate commit)

Remove from `server/src/bot.rs`:
- `BLUE_ATTACK_EAST/WEST`, `RED_ATTACK_EAST/WEST` constants
- `attack_route_for`, `next_attack_waypoint_stateful`
- `cta_waypoint_idx` field + its init + reset logic
- `WAYPOINT_ARRIVED_RADIUS` constant
- Debug `info!` bot-state logger

Reinstate the original direct-to-base pull at `bot.rs:252` so the
commit compiles. (Phase 2 will replace it with the flow sample —
the two-step keeps bisect clean.)

**Keep** (orthogonal, correct): spring `+=` fix, baseline terrain
repel (already at pre-tune defaults), difficulty-selector UI,
HTML ship labels, in-match Quit button.

## Phase 1 — FlowField module (own commit)

New file `server/src/bot_pathfinder.rs`. Struct + `build` + `sample`.

**Unit tests:**
- Open arena (flat water everywhere): integration increases
  monotonically with Euclidean distance from goal (±1 cell)
- Island between source and goal: cells behind island have
  strictly higher integration than the straight-line cost
- Unreachable pocket: integration = `IMPASSABLE_COST`
- Inflation: a 1-cell obstacle blocks a 7-cell region (3-cell
  radius around it)
- Determinism: two builds with same terrain produce byte-
  identical `integration` and `flow`
- Symmetry sanity: `flow_to_blue.sample(Vec2::ZERO)` and
  `flow_to_red.sample(Vec2::ZERO)` point in roughly opposite
  directions (dot product < -0.3)
- Sample on blocked cell returns `None`

## Phase 2 — integration + instrumented acceptance (own commit)

Server: add `flow_to_blue`/`flow_to_red` fields + build/clear
lifecycle hooks. Bot: add `cta_flow_direction`, `cta_stuck_since_tick`,
sample in outer update, consume in inner update, stuck-detection
logic.

**Instrumentation (dev-only, `#[cfg(debug_assertions)]`):** add two
counters that increment during the match and log on match-end:
- `bots_in_contested_zone`: number of teammate and enemy bots that
  enter a 300 m radius around `(0, 0)` during the match
- `spawn_terrain_deaths`: `Fate::Remove(DeathReason::Terrain)` fires
  for any bot within 5 s of its last spawn

**Acceptance** (measured by the counters, not observation):
- Across 3 consecutive matches: ≥ 2 teammate + ≥ 2 enemy bots
  enter the contested zone within 60 s of match start
- Across 3 consecutive matches: `spawn_terrain_deaths == 0`

Merge when both hold. No separate "Phase 3 merge" — once the
counters pass, land it.

## Non-goals

- Mid-match terrain-damage rebuild. Short-range terrain repel
  catches craters; revisit only if playtest shows otherwise.
- Free Roam pathfinding. No global pressure to drive into land.

## Files

| File | Change | Rough LOC |
|------|--------|-----------|
| `server/src/bot_pathfinder.rs` | NEW — FlowField + tests | ~180 |
| `server/src/server.rs` | flow_to_blue/red fields + lifecycle hooks | ~25 |
| `server/src/bot.rs` | Phase 0 revert (-100) + Phase 2 add (+30) | net -70 |

Three commits (Phase 0, Phase 1, Phase 2), one PR.

## References

- `server/src/noise.rs:12,23` — deterministic SEED, noise_generator
- `common/src/terrain.rs:25,44` — SCALE, SAND_LEVEL
- `common/src/ticks.rs:7` — 10 Hz tick rate
- `server/src/match_state.rs:30-48` — ArenaLayout::DEFAULT
- `server/src/bot.rs:252,461-506` — integration sites
- Emerson, *Game AI Pro* vol. 1 ch. 23 —
  [gameaipro.com][emerson]
- Patel, Red Blob Games — [redblobgames.com][redblob]
- Reynolds, Steering Behaviors — [red3d.com][red3d]
- Prior plans: `cta-bot-ai-improvements.md` (tuning, failed),
  `cta-carve-arena.md` (full carve, reverted),
  `cta-bot-targeting.md` (defense selection, layer on top)

[emerson]: http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter23_Crowd_Pathfinding_and_Steering_Using_Flow_Field_Tiles.pdf
[redblob]: https://www.redblobgames.com/pathfinding/tower-defense/
[red3d]: https://www.red3d.com/cwr/steer/
