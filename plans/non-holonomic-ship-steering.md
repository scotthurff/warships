# feat: Non-holonomic ship steering layer for CTA bots

> **Status:** Planned — 2026-04-15
> **Scope:** One steering layer on top of the flow field. No routing
> changes. ~80 LOC across `bot.rs`, `server.rs`, `world_physics.rs`
> (controller + four instrumented counters).
> **Motivation:** The flow field routes correctly; bots keep dying
> on terrain because ship kinematics (turn rate, forward momentum)
> can't execute cell-granular headings fast enough. Inflation and
> look-ahead tweaks don't fix this — the agent's physics don't
> match the flow's spatial assumptions. Need a controller layer
> that adapts to the ship's turn radius and stopping distance.

## Retrospective — what went wrong in the flow-field plan

I went into the flow-field implementation with 85% confidence citing
StarCraft 2 / Supreme Commander / Planetary Annihilation. All of
those games use flow fields successfully. I missed that **their
units are holonomic** — they can change heading instantly. A
StarCraft marine pivots 180° in one frame. Our ships can't.

The kinematic reality in `common/src/transform.rs:65-81`:

```rust
// Longer boats turn slower.
EntityKind::Boat => 0.125 + 20.0 / data.length,
```

**Turn rate per second by ship class** (delta_seconds = 1.0):

| Ship | Length | Turn rate | 90° turn |
|------|--------|-----------|----------|
| Lürssen (L1) | 30 m | 0.79 rad/s = 45°/s | 2.0 s |
| Destroyer (L5) | 120 m | 0.29 rad/s = 17°/s | 5.4 s |
| Iowa (L10) | 270 m | 0.20 rad/s = 11°/s | 7.9 s |

An Iowa needs **4 flow cells** (at 25 m/cell) to execute a 90° turn.
During those 4 cells it's committed to its old heading. If any of
those cells contains terrain (or borders an inflated cell), the ship
hits land before the turn completes.

Acceleration is similarly slow: `max_accel = delta_seconds ×
max_speed / 3`. At 10 Hz / 20 m/s max, that's 0.67 m/s² decel.
Going from cruise (13 m/s) to maneuver speed (3 m/s) takes ~15
seconds. "Just slow down when you see terrain" only works if you
start slowing WELL in advance.

**This is a standard problem in autonomous vehicle + ship control
literature.** The plan's reviewers didn't flag it because they were
focused on routing correctness. I should have researched agent
kinematics. That was the miss.

## Problem statement

Flow field routing is correct. Bots sample valid headings. The bot
still dies because its ship can't execute the heading commanded by
the flow before translating into terrain.

Specifically, two failure modes during gameplay:

1. **Cell-boundary overshoot.** Bot approaches a cell boundary at
   13 m/s. The next cell's flow direction differs by 45-90°. Bot
   can only rotate 20-30° during the 2-3 ticks it takes to cross
   the boundary. Ship drifts into an adjacent cell — which is often
   inflated terrain (blocked) or actual land.

2. **Incorrect-turn commitment.** Bot is moving at cruise toward a
   waypoint. Flow changes direction sharply at an upcoming cell.
   Bot starts turning but forward momentum carries it past the
   turn point. Arrives off-path in terrain.

Both failures have the same root: **turn rate × ship speed × cell
size mismatch.** The flow field is spatially correct but temporally
unsolvable for non-holonomic agents at cruise speed.

## Research

### Academic + industry references

- **Pure Pursuit**, Coulter 1992 — target-point tracking for non-
  holonomic robots. Picks a "look-ahead point" along the path;
  computes the curvature needed to arc to it. Widely used for AVs.
  Proven to work with **velocity-adaptive look-ahead**: LAD scales
  with current speed so high-speed agents see further ahead.
  [MathWorks Pure Pursuit reference][mathworks-pp] confirms this
  and the [dynamic LAD research note][rg-lad] explicitly says
  "LAD is automatically adjusted by the robot velocity."

- **Non-holonomic path planning with potential fields** (Masoud et
  al, [smooth path with curvature constraints][ref-masoud]). Key
  finding: **streamlines with curvature constraint + pure pursuit
  switching** is the documented way to combine potential/flow
  fields with ship-class kinematics. Exactly our situation.

- **Fast marching with non-holonomic constraints** (Gomez et al,
  [smooth path planning for non-holonomic robots using fast
  marching][ref-gomez]). "Works over a smooth vector field, allows
  the simple introduction of non-holonomic constraints." Vindicates
  the core approach of flow-field + kinematic adapter layer.

- **RTS with flow-field + variable agent size**, jdxdev's [*RTS
  Pathfinding 3: Variable agent size*][jdxdev-3]. The series author
  explicitly calls out variable unit sizes as one of the hardest
  problems. Their solution uses per-size flow fields. We have 10
  ship levels — 10 fields is too many. The alternative they
  discuss: **size-largest inflation + per-agent speed adaptation**.

- **"Three-mode" AV control**, [collision avoidance paper][ref-pmc].
  Modes: following / braking / emergency steering. The transition
  is based on relative distance to obstacle. Directly maps to our
  need for speed throttling near terrain.

### Why Pure Pursuit is the minimum-viable answer for a game

The literature has MPC, SMC, fuzzy PID, RL-tuned controllers. All of
them are solving the autonomous-car problem where cross-track error
in millimeters matters. Our target is "don't die to terrain in a
5-minute naval match for kids." The difference is **orders of
magnitude** in precision requirement.

Pure Pursuit with a **velocity-adaptive look-ahead** plus a **turn-
triggered speed throttle** is the smallest thing that solves the
actual problem:

- Look-ahead scales with speed → fast ships plan further ahead
- Speed throttles when required turn exceeds what the ship can
  execute in the look-ahead distance

That's two tunable numbers (look-ahead seconds, turn-to-throttle
slope) and ~40 LOC. Order-of-magnitude simpler than MPC and
sufficient for gameplay.

### What won't work and why

- **Pure look-ahead sampling alone** (my last attempt). Moves the
  sample point but doesn't address forward momentum. Bot still
  crashes when the look-ahead point's heading requires a turn the
  ship can't execute.
- **More inflation**. Reduces navigable cells. Doesn't change that
  the bot still hits the inflated boundary at speed.
- **Per-ship-class flow fields**. Too many fields (one per Level
  × per goal = 20), memory OK but complexity not worth it vs. the
  universal "slow-down-in-tight-spots" approach.
- **Smaller cells (12.5 m)**. Quadruples memory + compute for the
  flow field. Halves cell-crossing time but doesn't address the
  multi-second turn rate. Treating a symptom.
- **Make bots holonomic (override turn rate)**. Violates the game's
  class-balance and "picking a bigger ship = easier" design. Also
  looks wrong visually — Iowas pirouetting like Corvettes.

## Design

Two changes in `server/src/bot.rs`, both in the outer update's
flow-field sampling block.

### 1. Velocity-adaptive look-ahead time

Replace the fixed `LOOKAHEAD_SECS = 1.5` with a per-ship value
scaled by the time the ship takes to complete a 90° turn:

```rust
// Look-ahead seconds scale with turn-rate inverse. Slow-turning
// ships (Iowa) need more time to plan ahead than fast-turning
// ones (Corvette). Formula mirrors common/src/transform.rs:70:
//   turn_rate_per_sec = 0.125 + 20.0 / data.length
// Time for a 90° turn = (PI/2) / turn_rate.
let turn_rate = 0.125 + 20.0 / data.length;
let time_for_90_deg = std::f32::consts::FRAC_PI_2 / turn_rate;
// 0.6× the 90° turn time: tuned so an Iowa sees ~4.8 s ahead,
// matching its 4-cell turn commitment at 25 m cells / 13 m/s.
// Clamped to [1.0, 5.0]: below 1.0 the trace collapses inside a
// single cell; above 5.0 the look-ahead reaches past meaningful
// tactical context.
let lookahead_secs = (time_for_90_deg * 0.6).clamp(1.0, 5.0);
```

For an Iowa: `lookahead_secs ≈ 4.8 s` → at 13 m/s the trace covers
62 m ahead. For a Lürssen: `≈ 1.2 s` = 16 m. Each ship plans over
its own turn budget.

### 2. Multi-cell forward trace

A single look-ahead sample sees the flow direction at the trace
endpoint — but the worst turn on the projected path is often in
the middle. If the ship has to turn sharply at second 2.4 and
straighten out by second 4.8, a single sample at the endpoint
shows benign flow and the throttle never fires. The ship commits
to full speed into a bend it can't execute.

Sample the flow at `TRACE_SAMPLES = 4` evenly-spaced points along
the projected arc, and use the maximum heading delta across all
samples:

```rust
// N = 4: enough to catch a turn straddling the midpoint of the
// trace without over-sampling. Cost budget: ~40 bots × 4 samples
// × 10 Hz = 1.6k flow lookups/sec — negligible given the flow
// field is a flat cell array indexed by (x, y) → usize.
//
// Short ships (Lürssen, lookahead ≈ 1.2 s ≈ 16 m) have all 4
// samples land within the same or adjacent flow cells. That's
// fine: the loop degenerates to sampling the immediate cell a
// few times, which costs nothing and keeps the code path
// uniform across ship classes.
const TRACE_SAMPLES: usize = 4;

let heading_angle = Angle::from(heading);
let mut worst_turn_rad: f32 = 0.0;
let mut blocked_samples: u32 = 0;

for i in 1..=TRACE_SAMPLES {
    let t = lookahead_secs * (i as f32) / (TRACE_SAMPLES as f32);
    let sample_pos = pos + heading * speed * t;

    // Blocked-cell handling: if the sample lands in inflated
    // terrain (flow undefined or near-zero length), SKIP the
    // sample rather than substituting a stand-in direction.
    // Substituting (e.g. falling back to the current-position
    // flow) papers over exactly the tight-corridor case the
    // trace exists to catch — a trace projecting into land
    // would silently look benign. Count blocked samples instead;
    // Section 3 forces slow_factor to floor when the majority
    // of samples come back blocked, treating that as the signal
    // "ship is headed into terrain."
    let Some(flow) = flow_field
        .sample(sample_pos)
        .filter(|f| f.length_squared() > 0.01)
    else {
        blocked_samples += 1;
        continue;
    };

    let flow_angle = Angle::from(flow);
    // Wrap-normalize the delta into [-pi, pi] before taking
    // absolute value. Raw f32 subtract-then-abs breaks at the
    // ±pi seam: heading = 170°, flow = -170° yields a 340°
    // "turn" instead of 20°. Use the Angle type's signed
    // subtract (wraps into [-pi, pi] by construction) — pattern
    // reference: common/src/transform.rs:65 and :81.
    let signed_delta_rad = (flow_angle - heading_angle).to_radians();
    let delta = signed_delta_rad.abs();
    if delta > worst_turn_rad {
        worst_turn_rad = delta;
    }
}

let majority_blocked = blocked_samples * 2 > TRACE_SAMPLES as u32;
```

`worst_turn_rad` is what the throttle uses — the tightest turn the
ship would need to execute anywhere along the look-ahead window,
not just at its endpoint.

### 3. Turn-triggered speed throttle

If the worst turn exceeds what the ship can rotate through during
the look-ahead window, reduce `velocity_target` proportionally:

```rust
let turn_budget_rad = turn_rate * lookahead_secs;

// over_budget crosses zero at 50% of budget — the ship starts
// throttling BEFORE it runs out of turn capacity, mirroring the
// "three-mode" AV controller's following→braking transition.
// Tuned so an Iowa at a 60° turn (1.047 rad) vs. 0.96 rad budget
// throttles to 59% of max speed. Higher than 0.5 here means
// ships throttle only at the edge of physical limit, leaving no
// correction margin for the next tick's sample drift.
let over_budget = (worst_turn_rad / turn_budget_rad - 0.5).max(0.0);

// 0.7: slope of the throttle ramp. At 1.0× budget → ~0.65 slow;
// at 2.0× budget → ~0.3 (floor). Steeper cliff-drops on minor
// excess; shallower lets too-fast ships drift into terrain.
// 0.3 floor: ships below ~3 m/s can't turn effectively
// (max_accel in common/src/transform.rs:96 caps angular
// response at low speed). Keep enough forward momentum to steer.
let slow_factor = if majority_blocked {
    // Trace projected into land — trust that signal over the
    // computed turn budget. Floor speed regardless of whether
    // the surviving samples happened to read benign deltas.
    0.3
} else {
    (1.0 - over_budget * 0.7).clamp(0.3, 1.0)
};

// One-tick latch: outer update computes this; inner update
// (bot.rs:435) reads it on the NEXT tick. The `pending_` prefix
// signals the tick lag at both call sites, distinguishing it
// from immediately-consumed state like cta_flow_direction.
bot_state.pending_speed_scale = slow_factor;
```

The inner update at `bot.rs:435` already sets:

```rust
velocity_target: data.speed * match Difficulty::get_global() {
    Captain => 0.80, Admiral => 0.88, FleetCommander => 0.95,
},
```

Add one multiplication:

```rust
velocity_target: data.speed * self.pending_speed_scale * difficulty_multiplier,
```

**Math sanity check.** Iowa approaches terrain where the trace
finds a 60° turn at sample 3 of 4 (2.4 s ahead). Samples 1, 2, 4
show benign 10° deltas; `worst_turn_rad = 1.047` (the 60°).
`turn_budget = 0.20 × 4.8 = 0.96 rad`. `over_budget = 1.047/0.96 -
0.5 = 0.59`. `slow_factor = 1.0 - 0.59 × 0.7 = 0.59`. Ship
throttles to 59% of max — say 7.7 m/s. The 60° turn now executes
over 4.8 s at 7.7 m/s = 37 m of translation instead of 62 m at
13 m/s. Fits. Ship survives.

For a 45° worst-turn — within budget — `over_budget = 0`,
`slow_factor = 1.0`. No throttle. Fast ships stay fast in open
water.

**Why the trace matters here.** Under single-point (endpoint-only)
sampling, sample 4 reads 10° — benign — and `slow_factor` stays
at 1.0. The ship commits to 13 m/s into the 60° bend at sample 3.
Same failure mode as today. The 4-sample trace is what makes the
throttle actually fire when it needs to; the adaptive lookahead
time is what makes the trace cover the right window for each
ship class.

### What this doesn't do

- Does not change the flow field (already correct).
- Does not change inflation / goal-clear (current values stay).
- Does not introduce path following, waypoint tracking, or
  smoothing.
- Does not add per-class flow fields.

## Acceptance

Instrumented counters (extending the existing `cta_contested_zone_visitors`):

- `cta_bot_terrain_deaths: u32` — increments on
  `Fate::Remove(DeathReason::Terrain)` for bot-owned boats during
  the match.
- `cta_bots_enemy_base_reached: HashSet<PlayerId>` — bots whose
  alive position entered within 250 m of the enemy base.
- `cta_bot_ticks_throttled: u32` — increments every tick a bot's
  `pending_speed_scale < 0.9`. Divided by total bot alive-ticks
  on match end to get a throttle-rate percentage. Catches the
  regression mode the two criteria above miss: bots don't die,
  but crab through open water at 30% speed because the throttle
  is papering over a routing bug.
- `cta_bot_alive_ticks: u32` — total tick-count across all alive
  bots during the match, denominator for the throttle rate.

All four logged on `MatchEvent::MatchEnded`.

**Pass criteria** (all three must hold across 3 consecutive matches):

1. **Terrain deaths per match ≤ 2** (down from ~5-10 currently).
   Zero is unrealistic for a game with dynamic enemy-pushing;
   "rare" is the goal.
2. **At least 1 bot per team reaches the enemy base per match.**
   Confirms routing works end-to-end.
3. **Throttle rate ≤ 40%** (`cta_bot_ticks_throttled` /
   `cta_bot_alive_ticks`). Above this means the throttle is
   running constantly — ships crawling through open water instead
   of actually turning hard in tight spots. Passing (1) and (2)
   while failing (3) means the fix worked by making bots too
   slow to die, not by teaching them to steer. That's a hidden
   failure and the gate must reject it.

**Secondary verification** (not a pass criterion — must still be
sanity-checked before declaring complete):

- Confirm `cta_stuck_ticks` reset still fires for bots floored
  at `pending_speed_scale = 0.3`. The existing check in the
  outer update uses a speed-not-position threshold, so low
  speed should still trip it; this catches the regression where
  a floored ship idles indefinitely against land. Acceptance
  signal: in the first post-merge match that contains at least
  one bot wall-hugging for 30+ ticks, `cta_stuck_ticks` must
  trigger at least one reset. If it doesn't, the stuck detector
  needs its own patch before shipping.

**Hard decision gate:** if both don't hold after this change, we
stop. Options at that point:
- Revert the flow-field work entirely; go back to direct vectors
  with the Phase 0 baseline.
- Accept current behavior as "good enough" (bots are imperfect,
  game is kid-friendly, nobody ranks the AI).
- Bigger structural change: per-class flow fields, arena carve,
  scripted hand-placed island geometry. All require their own
  plan.

This is NOT another "one more tweak" iteration. If the pass
criteria fail, the next move is explicit, not incremental.

## Implementation

Single commit on top of the current `flow-field-pathfinding` branch.

1. Add `pending_speed_scale: f32` field on `Bot` struct, default 1.0.
2. In the outer update's flow-sample block:
   - Compute `turn_rate` and `lookahead_secs` from `data.length`.
   - Run the 4-sample forward trace; for each sample, look up the
     flow and (if blocked or near-zero) fall back to
     `bot_state.cta_flow_direction` (the current-position flow
     already sampled upstream).
   - Use wrap-normalized angle delta against the ship's heading
     (via `Angle::sub` which returns [-pi, pi] by construction).
   - Take the max delta across samples as `worst_turn_rad`.
   - Compute `slow_factor` via the throttle formula; store in
     `bot_state.pending_speed_scale`.
3. In the inner update's `velocity_target` expression (line ~435),
   multiply by `self.pending_speed_scale`.
4. Add `cta_bot_terrain_deaths` counter on Server — hook into the
   bot's death path (grep for `DeathReason::Terrain` in
   world_physics.rs for the fire site). Only increment for
   bot-owned boats.
5. Add `cta_bots_enemy_base_reached` counter (extend the existing
   contested-zone sampling block).
6. Add `cta_bot_ticks_throttled` and `cta_bot_alive_ticks`
   counters. Increment both in the outer update: `alive_ticks`
   for every alive bot on every tick; `ticks_throttled` when
   `pending_speed_scale < 0.9`.
7. Log all four on `MatchEvent::MatchEnded`, computing and
   reporting the throttle-rate percentage for quick gate reads.

## Non-goals

- Per-ship-class flow fields.
- Pure Pursuit with carrot-at-distance-on-path (true PP needs a
  path, we have a vector field — adapting it is a bigger change).
- MPC or SMC controllers.
- Rubber-band / path-smoothing on top of the flow.
- Reversing direction to escape tight spots. Existing stuck
  detection covers that.
- Free Roam bots (this stays CTA-only).

## Files

| File | Change | LOC |
|------|--------|-----|
| `server/src/bot.rs` | `pending_speed_scale` field, lookahead-time calc, 4-sample forward trace with blocked-cell fallback, wrap-safe angle delta, throttle formula, inner-update scale application, per-tick throttle-rate counter increments | ~55 |
| `server/src/server.rs` | `cta_bots_enemy_base_reached` + `cta_bot_terrain_deaths` + `cta_bot_ticks_throttled` + `cta_bot_alive_ticks` counters; match-end logs with computed throttle-rate percentage | ~20 |
| `server/src/world_physics.rs` | Hook the terrain-death counter (one-line increment inside the `Fate::Remove(DeathReason::Terrain)` block when entity is bot-owned) | ~5 |
| **Total** | | **~80 LOC** |

One commit, on top of the existing `flow-field-pathfinding`
branch.

## References

[mathworks-pp]: https://www.mathworks.com/help/robotics/ref/purepursuit.html
[rg-lad]: https://www.researchgate.net/figure/258177250_fig20_Figure-20-Velocity-and-the-look-ahead-distance-path-tracking-and-obstacle-avoidance
[ref-masoud]: https://www.researchgate.net/publication/291620963_Smooth_Path_Planning_around_Elliptical_Obstacles_Using_Potential_Flow_for_Non-holonomic_Robots
[ref-gomez]: https://www.researchgate.net/publication/224459807_Smooth_path_planning_for_non-holonomic_robots_using_fast_marching
[jdxdev-3]: https://www.jdxdev.com/blog/2021/12/07/rts-pathfinding-3-variable-agent-size-smoke-tests-navmesh-fixes/
[ref-pmc]: https://pmc.ncbi.nlm.nih.gov/articles/PMC11359412/

- Coulter, R. C. (1992). Implementation of the Pure Pursuit Path
  Tracking Algorithm. Carnegie Mellon.
- `common/src/transform.rs:65-81` — turn-rate formula
- `common/src/transform.rs:96-116` — velocity physics
- `server/src/bot.rs:435` — current `velocity_target` site
- `server/src/world_physics.rs:208` — `Fate::Remove(DeathReason::Terrain)`
- Current branch state: `flow-field-pathfinding`, 4 commits ahead
  of main, 30 tests passing.

## Decision framing — honest about what we're committing to

This is the **last** attempt at making flow-field + current ship
physics work cleanly. The Pure Pursuit + speed throttle combo is
documented as the standard non-holonomic-agent fix in the papers
I've cited. If it fails the pass criteria above, the evidence
is then clear: flow fields need either per-class variants (bigger
refactor) or a proper path-following controller (MPC/Stanley-
level, much bigger change) — or we stop and pick one of the non-
flow-field options in the decision gate.

No more "one constant bump" iterations. Either the acceptance
criteria hit or we change approach.
