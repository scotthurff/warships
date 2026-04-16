# feat: Expand the CTA arena + sparsen terrain for navigability

> **Status:** Planned — 2026-04-15
> **Scope:** ~75 LOC across server arena/noise/flow-field code
> (plus a small common/terrain.rs helper if not already present).
> **Motivation:** The Phase 3b steering-layer gate failed match 1 at
> 376 terrain deaths and 80% throttle rate. The plan's decision gate
> explicitly rejected "one more controller tweak" and named
> structural alternatives. This is that structural alternative —
> making the arena itself work for ship kinematics, instead of
> asking the controller to solve physics that can't be solved at the
> current scale.

## Retrospective — why the steering layer alone couldn't save it

Phase 3b was implemented correctly: 4-sample forward trace, wrap-safe
angle math, turn-budget throttle. Measured match 1 result: the
throttle fires 80% of alive-ticks — meaning the controller IS seeing
danger and IS slowing ships down — yet bots still die 376 times in
one 5-minute match.

Root cause: **it's not a control problem, it's a scale problem.** The
current arena (1000 m base-to-base, with procedural terrain scattered
through the middle band) packs too much terrain into too small an area
for ships whose turning radius is 100+ m. The flow field routes
correctly; the steering layer throttles correctly; the bot still
dies because no route AND no speed can thread those gaps at ship
scale.

The honest fix is to give ships more room and less terrain — which is
what this plan does — not to keep tuning a controller that's already
doing its job.

## Prior attempt — what the reverted carve got wrong

Commit `ae1026a` ("Carve CTA arena flat to stop bots crashing") applied
a post-noise flatten to the ENTIRE 1500 m arena disk. It worked for
navigation but looked barren: the match played on a featureless blue
circle. Reverted in `b32bed0`.

This plan differs from that one on three axes:

- **Sparsen, don't flatten.** Reduce the density of blocking cells
  inside the corridor to ~30% of current, but preserve the rest as
  visible islands. Arena remains recognizably terrain-filled.
- **Corridor, not disk.** Only the blue→red base navigation corridor
  is sparsened. Perimeter terrain (outside the corridor) is
  untouched; the eastern/western map edges still have full
  procedural terrain for visual flavor and "edge of world" framing.
- **Bigger arena.** The previous carve kept the 1000 m base
  distance. This doubles it — ships have room to maneuver BEFORE
  engaging terrain geometry.

## Design

### 1. Expand the arena (2× base distance)

Current `ArenaLayout::DEFAULT` in `server/src/match_state.rs:40`:

```rust
pub const DEFAULT: Self = Self {
    blue_base: Vec2::new(0.0, 500.0),
    red_base: Vec2::new(0.0, -500.0),
    base_radius: 250.0,
};
```

Proposed:

```rust
pub const DEFAULT: Self = Self {
    blue_base: Vec2::new(0.0, 1000.0),
    red_base: Vec2::new(0.0, -1000.0),
    base_radius: 250.0,
};
```

Distance 1000 m → 2000 m. `base_radius` unchanged — bases stay the
same size, just farther apart. Ships have 2× the open water to
traverse and turn in before reaching the opponent.

### 2. Scale the world/visible radius

Current `World::target_radius` (in `server/src/world.rs:105`) scales
with total ship visual area, producing ~1400 m for 10 ships. At that
radius the current bases at (0, ±500) are well inside; moving them
to (0, ±1000) puts them near the boundary — ships at the base
already see the "edge of world."

Proposed: enforce a CTA minimum world radius of 3000 m while any
player is in CTA mode:

```rust
const CTA_MIN_WORLD_RADIUS: f32 = 3000.0;
```

3000 m = 1000 m base offset + 1000 m northward roam room + 1000 m
buffer. Gives ships space north of the blue base and south of the
red base (today they hit the border almost immediately).

### 3. Sparsen the corridor terrain

**Where:** a stadium (rectangle with semicircular caps) from blue base
to red base, half-width 700 m.

- Center axis: x = 0, y from −1000 to +1000.
- Half-width: 700 m (measured perpendicular to the axis).
- End caps: half-disks of radius 700 m centered at each base.

**How:** on CTA match start, walk a 20 m grid inside the stadium.
For each cell where altitude ≥ `SAND_LEVEL` (land), apply
`TerrainMutation::clamped(pos, -200, MIN..=Altitude(-8))` with
probability 0.7 (deterministic — see below).

**Determinism:** use a cheap positional hash of `(grid_x, grid_y)`
so the same terrain is sparsened the same way every match. Avoids
RNG-ordering dependence on match start.

**Hash: must mix both axes.** A naive `(x * K1) ^ (y * K2)` has a
blind spot at `x = 0` or `y = 0` — and the corridor center axis
is literally `x = 0`. Streaks of all-land or all-water would
appear exactly along the main navigation spine. Use a proper 2D
mixer like xxhash32's finalizer, FNV-1a over the two u32s, or
splitmix32 after combining:

```rust
// MurmurHash3 fmix32 over a combined u64 seed. Both axes
// contribute even when either is zero. ~70% flatten at cutoff.
fn stadium_keep(x: i32, y: i32) -> bool {
    let mut h = ((x as u32).wrapping_mul(0xcc9e2d51))
        ^ ((y as u32).wrapping_mul(0x1b873593)).rotate_left(15);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;
    (h & 0xFF) > 0xB3  // 0xB3/0xFF ≈ 0.3 survival
}
```

This hash passes a quick sanity check: at `x=0`, `h` still
depends non-trivially on `y` after two rounds of fmix; at
`y=0`, same for `x`. Single-axis streaks disappear.

Result: roughly 30% of the blocking islands inside the corridor
survive as visible sand/rock. 70% become water, opening navigable
paths between them. Ships can weave, the flow field has enough
open cells to route freely, bots with wide turn radius have room
to turn BEFORE committing.

**Mutation path: direct, not queued.** The sparsen pass runs
once at match start, applying ~4000 cell mutations in a single
loop. Do NOT use the `TerrainMutation::clamped` path that ships
use mid-game — that path collects into a mutex-guarded Vec
(`server/src/world_physics.rs:260`) that the next physics tick
drains. Feeding 4000 mutations through that at match start
would either contend the mutex or queue up and delay field
construction. Instead, apply directly to the `Terrain` buffer:

```rust
// server/src/cta_terrain.rs pseudocode
pub fn sparsen_cta_corridor(terrain: &mut Terrain, layout: &ArenaLayout) {
    for world_pos in stadium_cells(layout, 20.0) {
        if terrain.sample(world_pos).unwrap_or(Altitude::MIN)
            < terrain::SAND_LEVEL
        {
            continue; // already water
        }
        if !stadium_keep(world_pos.x as i32, world_pos.y as i32) {
            terrain.set_altitude(world_pos, Altitude(-8)); // direct write
        }
    }
}
```

If `Terrain` doesn't expose a direct `set_altitude(Vec2, Altitude)`,
add one as part of this change (it's a trivial wrapper over the
existing mutate_internal path — see `common/src/terrain.rs:567+`
for the mutation infrastructure). Direct-write is the correct tool
for match-start bulk-edit; the mutation queue exists to serialize
per-tick gameplay events.

**Outside the stadium:** terrain generation is untouched. The
eastern and western sides of the map (x ≲ −700 or x ≳ +700), plus
the arctic band at y ≥ 1250 and tropics at y ≤ −2250, keep their
full procedural islands. The world doesn't look flat; the
corridor just looks *sparser*, like an open channel with scattered
islands rather than dense terrain.

### 4. Keep terrain destructible (user: "less persistent")

No change. The existing `world_physics.rs` terrain-mutation path
already lets ships break sand they collide with. Keep that —
ships running into one of the surviving islands can still carve
through it at a cost, rather than being hard-walled.

This is the "less persistent" half of the user's request: terrain
is visible, but not immutable. A determined ship (or an explosion
cascade) can still reshape the map mid-match.

### 5. Scale the flow-field grid coverage

Current `GRID_SIZE = 128` at `CELL_SIZE = 25 m` → world range ±1600 m.
At the new 2× arena, the grid no longer covers the 3000 m world
radius. Ships north of y = +1600 get `None` from `FlowField::sample`,
fall back to direct-to-goal, and die to border/terrain without the
flow-field's routing help.

Proposed: **`CELL_SIZE = 50 m`, `GRID_SIZE = 128` unchanged** →
world range ±3200 m. Covers the new 3000 m world radius with
margin, at ZERO memory cost vs today.

The tempting alternative — `GRID_SIZE = 256` at 25 m cells — was
rejected. It quadruples memory (16k → 65k cells × 10 bytes = ~650 KB
per field × 2 = ~1.3 MB) and Dijkstra work for no meaningful
benefit: at ship turn radii of 100–270 m, 25 m flow-field
resolution is already over-sampled. 50 m cells match the actual
spatial grain at which bots can respond; the steering layer's
4-sample forward trace handles sub-cell accuracy within its
look-ahead window.

Flow-field `CELL_SIZE` is independent of the terrain's
`common/src/terrain.rs:25` `SCALE = 25 m` — flow samples terrain
by world position (not by cell index), so the two grids can
have different resolutions without resampling logic. Only the
flow field's own `world_to_cell` / `cell_to_world` helpers need
to see the new constant.

### 6. Keep Phase 3a/3b work for THIS commit — evaluate revert after gate passes

The flow field + steering layer stay in place for this commit, to
keep the change atomic and easy to revert if the structural
theory is wrong. Expected behavior after this change:

- **Flow field** routes ships through the now-open corridor with
  scattered islands. Fewer blocked cells → better integration
  values → cleaner flow directions.
- **Steering layer throttle** fires rarely (mostly open water).
  `worst_turn_rad` stays below the turn budget for most samples;
  `slow_factor = 1.0` most of the time.
- **Stuck detection** unchanged.

**Post-gate follow-up (NOT this commit):** if matches 1–3 show
throttle rate < ~5% — meaning the controller is a dead-weight
no-op — revert Phase 3b in a separate commit. Keeping unverified
code paths that pay a real per-tick cost is not "insurance"; it's
deadwood. The measurement determines the revert, not a framing
choice up front.

## What this does NOT do

- Does NOT reshape terrain biomes globally. Arctic and tropics
  bands are untouched outside the corridor.
- Does NOT lock terrain during matches. Ships can still break
  sand/ice — user explicitly asked for "less persistent."
- Does NOT add hand-placed islands. Terrain is still procedural;
  just sparsened inside the corridor.
- Does NOT change the bot AI, match FSM, scoring, or capture logic.
- Does NOT introduce a new CTA mode variant. Same CTA, larger map.
- Does NOT rebuild the flow field mid-match when ships carve new
  water through surviving islands. This is a pre-existing
  limitation (flow fields are built once at match start); the
  larger map makes it slightly more visible because corridors are
  longer, but the fix belongs in a separate plan if it matters in
  practice. Usually ships carve small gaps late in the match when
  routing is no longer decisive.

## Acceptance

All four match-end counters from Phase 3b stay in place. Gate
(all must hold across 3 consecutive matches):

1. **terrain_deaths ≤ 10 per match** (down from 376 at current scale).
   Goal is *rare*, not zero — procedural terrain can still trap the
   occasional unlucky spawn.
2. **≥ 3 bots per team reach the enemy base per match** (up from 1
   in the Phase 3b acceptance — previous gate barely passed this).
3. **throttle_rate ≤ 40% match 1, ≤ 25% by match 3.** (Down from
   80%.) The match-1 gate matches Phase 3b's original target — we
   don't know what the ratio will be with sparser terrain, and
   claiming 80 → 25 in one shot without math is weakly supported.
   Matches 2–3 should tighten the controller's firing rate as
   early-match spawn chaos settles. If match 3 throttle rate is
   still >25%, the sparsening ratio needs to go up (0.7 → 0.8+).
4. **Visual check (manual)**: terrain is still clearly visible on
   the map; the arena does not look like a flat blue disk. This is
   the guard against repeating the reverted-carve failure.

**Hard decision gate:** if terrain_deaths stay above ~30 per match
after this change, the structural theory is wrong and we stop.
Options at that point:

- Sparsen more aggressively (0.7 → 0.9+ flatten ratio)
- Widen the corridor (700 m → 900 m half-width)
- Accept current behavior as good enough and move on
- Rip the flow-field work entirely; direct-vector-only AI at this
  enlarged map scale might be adequate on its own

Not another controller iteration.

## Files

| File | Change | LOC |
|------|--------|-----|
| `server/src/match_state.rs` | `ArenaLayout::DEFAULT` base y ±500 → ±1000 | ~2 |
| `server/src/cta_terrain.rs` | NEW — `sparsen_cta_corridor` + `stadium_keep` hash + stadium iterator | ~50 |
| `server/src/server.rs` | Call sparsen during `build_flow_fields`; enforce `CTA_MIN_WORLD_RADIUS` on match start | ~12 |
| `server/src/bot_pathfinder.rs` | `CELL_SIZE` 25 → 50, comment update | ~3 |
| `server/src/world.rs` | Minimum-radius override for CTA | ~5 |
| `common/src/terrain.rs` | Expose `set_altitude(Vec2, Altitude)` helper if not already present | ~5 |
| **Total** | | **~75 LOC** |

One commit on top of `flow-field-pathfinding` branch. Estimate is
the honest count including stadium-traversal boilerplate, the
MurmurHash3 fmix, world-radius conditional, and likely 1–2 test
fixes when `GRID_SIZE`/`CELL_SIZE` constants change.

## Implementation sequence

1. `server/src/match_state.rs`: bump `blue_base.y` and `red_base.y`
   to ±1000. Confirm existing tests (`match_state::tests::*`) still
   pass — they reference base positions generically.

2. `server/src/bot_pathfinder.rs`: `CELL_SIZE` 25 → 50 (keep
   `GRID_SIZE = 128`). Confirm the 8 flow-field tests still pass —
   they build against goal positions at specific world coordinates,
   which should still land inside the grid at ±3200 m coverage.

3. `server/src/world.rs`: add `CTA_MIN_WORLD_RADIUS: f32 = 3000.0`
   and a check in `World::update` (or the tick loop) to clamp
   `self.radius = self.radius.max(CTA_MIN_WORLD_RADIUS)` when any
   player is in CTA mode. Use the same `suppress_statics` flag
   that already gates CTA-mode world behavior.

4. `server/src/cta_terrain.rs`: new module with:
   - `pub fn sparsen_cta_corridor(terrain: &mut Terrain, layout: &ArenaLayout)`
   - Walks a 20 m grid over the bounding box of the stadium
     (`|y| ≤ 1000 + 700`, `|x| ≤ 700`); filters with an
     inside-stadium predicate (rectangle band OR either endpoint
     half-disk)
   - MurmurHash3 fmix32 `stadium_keep(x, y)` for each land cell
   - Direct `Terrain::set_altitude(pos, Altitude(-8))` write — NOT
     the `TerrainMutation::clamped` queue path (per §3's mutation-
     path note). If `Terrain::set_altitude` doesn't exist, add a
     `pub fn` wrapper around the existing mutate internals in
     `common/src/terrain.rs` as part of this change.

5. `server/src/server.rs::build_flow_fields`: call
   `sparsen_cta_corridor` BEFORE the `FlowField::build` calls, so
   the flow field is built against the already-sparsened terrain.

6. Compile, run the 30 existing tests. Fix any regressions.

7. Run a live 5-min match, observe the match-end log:
   `steering — terrain_deaths=N enemy_base_reached=M throttle_rate=R%`
   against the gate above.

## References

- `plans/non-holonomic-ship-steering.md` — Phase 3b (controller layer,
  failed at current scale)
- `git show ae1026a` — reverted "flatten-everything" carve (for the
  lessons it teaches about visual barrenness)
- `server/src/noise.rs` — current noise generator; unchanged
- `common/src/terrain.rs` — `SCALE = 25m/cell`, `SAND_LEVEL`,
  `TerrainMutation::clamped` (used by ships to break sand/ice;
  reused here for the corridor sparsen pass)
- `server/src/world.rs:105` — `World::target_radius` (ship-visual-area
  sizing; CTA overrides it with a floor)
- `server/src/bot_pathfinder.rs:36-71` — flow-field grid constants
- `server/src/match_state.rs:32-44` — `ArenaLayout::DEFAULT`

## Decision framing

This is the structural fix Phase 3b's gate pointed at. It's
narrower in scope than the three structural options the gate
listed — it doesn't add per-class flow fields, doesn't script
hand-placed geometry, doesn't build a real path-following
controller. It just gives the ships more room and less terrain
where the fight actually happens.

If this doesn't work, the diagnosis is clear: *procedural terrain
at any density is incompatible with our ship kinematics*, and the
next move is either hand-scripted geometry or accepting that
bots are imperfect in a kid-friendly game where nobody ranks the AI.
