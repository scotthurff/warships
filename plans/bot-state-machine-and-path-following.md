# feat: Replace boids-aggregation bot AI with state machine + path-following

> **Status:** Planned — 2026-04-15
> **Scope:** NEW `server/src/bot_behavior.rs` module + `bot.rs`
> refactor + small `bot_pathfinder.rs` helper. Replaces the
> per-tick movement-vector aggregation with a Pure Pursuit path
> follower driven by a finite state machine with hysteretic
> transitions.
> **Motivation:** Five consecutive iterations of patching the
> force-aggregation AI (Phase 0 → Phase 5) have failed to produce
> bots that reach the enemy base. Each patch addressed a symptom.
> The pattern in the measurements makes the diagnosis unambiguous:
> the failure is architectural, not tunable.

## Retrospective — why every patch has failed

| Phase | Change | Target metric | Result |
|---|---|---|---|
| 0 | Direct-vector AI | navigate terrain | bots beach themselves |
| 1–3a | Flow fields + inflation + goal-clear | routing correct | routing worked, bots still died |
| 3b | Steering layer (4-sample trace + throttle) | stop terrain deaths via controller | throttle fires 80%; bots still die 376×/match |
| 4 | Sparsen arena + 2× base distance | remove terrain as the blocker | terrain_deaths 376 → 12 (31×); enemy_base_reached still 0 |
| 5 | Bump objective weight 6.0 unconditional | force commitment past combat | terrain_deaths regressed 12 → 83; enemy_base_reached still 0 |

The underlying pattern in every failure:

`bot.rs:150-326` computes `movement = terrain_repel + enemy_spring +
friend_spring + flow_pull` each tick, sets
`direction_target = Angle::from(movement)`, and returns.

This is **Reynolds 1987 boids aggregation**. It was state-of-the-art
for flocks in a 1987 SIGGRAPH paper. For a 2026 goal-directed agent
in the presence of enemies it produces:

- **Direction jitter** — when forces near-cancel, the small residual
  vector's angle is dominated by noise. direction_target swings
  wildly tick-to-tick.
- **Force-cancellation twirl** — EDI's velocity trace during the
  Phase 5 match: `+9.9, -13.1, +9.1, +15.0, -9.4, +4.8, -5.6` m/s.
  Ship decelerates from forward motion as heading flips, then tries
  to accelerate into the new direction, heading flips again.
- **Parked-at-engagement-range** — enemy springs pull toward 200 m
  orbit distance from each visible enemy. At midfield scrum, 3–5
  concurrent springs plus mutual teammate separation produce a
  stable local minimum. Bots sit there.
- **No multi-step reasoning** — bot cannot plan "go around this
  island, then engage." Every tick is a fresh local decision.
- **No commitment** — no memory of "I was heading north 200 ms ago,"
  so nothing resists oscillation.

The Phase 5 post-mortem nails the philosophical point: **a stronger
objective weight didn't unlock the path to the enemy base.**
Something else is blocking it. Something else is "I have no plan,
just a direction gradient, and the gradient flips when the world
flips." That's what we replace.

## Architectural diagnosis

The bot needs THREE things the current code does not have:

1. **Commitment to a path over time.** A plan that survives the next
   tick's local perturbations. Local forces can DEFLECT the ship
   around an obstacle, but cannot REDIRECT its high-level goal.

2. **Named states with explicit transitions.** Bot is either
   Transiting (moving toward objective), Engaging (close enough to
   an enemy to shoot, but still path-bound), Defending (own base
   under attack, stay local), or Committing (final push, ignore
   detours). Not a soup of forces whose resultant's angle is the
   behavior.

3. **Hysteresis on every transition.** The current defense-mode flap
   (capture_progress crosses 5000 ms → defending toggles → flow
   direction flips → ship twirls) is the canonical example of
   hysteresis-missing. Every state transition needs a "hold for N
   ticks" or "asymmetric threshold" guard.

## Research

One load-bearing citation:

- **Pure Pursuit** (Coulter 1992, CMU Robotics Inst.). Phase 3b
  implemented the throttle piece only — pretending the flow
  field's single-cell gradient was the path, then adapting the
  velocity to the heading error. That's a degenerate
  1-waypoint case. The full algorithm computes a multi-waypoint
  path, picks a look-ahead "carrot" point on the path at
  velocity-adaptive distance, and steers to the carrot. That
  carrot-on-path geometry is what §Path following implements.

Everything else in this plan is straightforward engineering:
state machine with hysteretic transitions, input/output data
structures, unit tests. The architectural claim ("per-tick
decision making without commitment produces the measured
failures") is supported by the Phase 0-5 retrospective table
and by EDI's velocity trace, not by external citation.

## Design

### Three structural changes

1. **`bot_pathfinder.rs::trace_path`** — new function. Given
   start and goal, returns a `Vec<Vec2>` of ~5–8 waypoints by
   walking the flow field from start toward goal with a fixed
   step size. Cheap: O(waypoint_count) field samples.

2. **`bot_behavior.rs`** (new module) — finite state machine, path
   storage, Pure Pursuit carrot follower. Self-contained. Unit-
   testable (pure functions for state transitions and carrot
   geometry).

3. **`bot.rs` inner update** — strips the movement-vector
   aggregation (lines 150–326) down to: (a) a call into
   `bot_behavior::tick` that returns `(direction_target,
   velocity_target_scale)`, (b) the existing aim/firing logic
   (untouched), (c) the submarine submerge hysteresis
   (untouched). Local forces (terrain close-range repel, enemy
   proximity dodge) become small perturbations applied BY the
   behavior module, NOT the top-level decision.

### State machine

Three states. Committing is NOT a separate state — it is a boolean
mode on Transiting (`Transiting { committing: bool }`). The original
draft had it as a 5th state, but the only differences from Transiting
are three parameter overrides (velocity, aim filter, look-ahead). A
mode flag captures that without growing the transition graph.

```
              respawn
                ↓
             Spawning
                ↓ alive
             Transiting { committing } ←────────┐
              ↓    ↑                            │
              │    │ within engagement range    │ enemy out of
              │    │ AND target not captured    │ sensor range
              │    ↓                            │ for 40 ticks
              │  Engaging ──────────────────────┘
              │    │
              │    ↓ (see Defending entry condition below)
              │    ↓
              │  Defending
              │    │
              │    ↓ threat cleared for 5 continuous ticks
              ↓    ↓
           back to Transiting
```

Committing is a flag ON Transiting, toggled every tick by the
predicate `within(700 m of enemy base) AND target == enemy base`.
It affects three things the tick function reads:
- velocity target override: `1.0` (no Phase-3b throttle; see §Phase 3b composition)
- aim filter: only aim at enemies within 300 m
- carrot look-ahead multiplier: `2.0×` instead of `1.0×`

#### State semantics

- **Spawning**: no control output. Transitions to Transiting as soon
  as the player's status flips to Alive. No tick-count gate — the
  entry is event-driven (status change), not timer-based.

- **Transiting**: following the current path toward the current
  `target` (enemy base, or own base if Defending — but during
  Defending the state is literally `Defending`, not `Transiting`).
  Carrot at look-ahead distance ahead on the path (§Path following).
  Velocity target = full speed × difficulty × Phase 3b throttle
  (composed per §Phase 3b composition). Local terrain repel
  perturbs the carrot direction by ≤ 20° if terrain is within 1
  ship-length of the ship's nose; otherwise no perturbation. When
  `Transiting.committing == true`, the velocity/aim/look-ahead
  overrides above apply.

- **Engaging**: same path as Transiting but with a SHORTER carrot
  look-ahead (0.5× the Transit value) so the ship can curve around
  a target within engagement range without losing path progress.
  Aiming + firing logic (unchanged from current bot.rs:328–431)
  decides when to shoot. Exit condition: closest enemy out of
  SENSOR range for 40 consecutive ticks (4 s at 10 Hz; 20 ticks
  in the first draft was too short given mk48's sensor-flicker
  at range can last 3-4 s on its own). "Out of sensor range" is
  defined precisely as `closest_enemy.is_none()` in the bot's
  per-tick contact scan.

- **Defending**: path target = own base center. Entry condition:
  `own_base_capture_progress > 10_000 ms` AND the bot is one of
  the top-3 closest teammates to own base. **Team-size
  short-circuit:** if fewer than 3 teammates are alive, "top-3
  closest" degenerates to "any alive teammate" — every surviving
  teammate defends. State EXIT: `capture_progress < 3_000 ms` for
  5 continuous ticks. Asymmetric thresholds (10 s entry, 3 s exit)
  are the defense-flap fix.

- **Retreating**: **explicitly NOT in this plan.** HP-based retreat
  is tempting ("bots with <25% HP head home") but it's
  orthogonal to the state machine this plan introduces. If the
  state+path design ships and bots still rush-die at low HP, that
  earns its own plan. The prior-draft transition-diagram reference
  to Retreating was removed.

#### Target invalidation

The target (enemy base for attackers, own base for Defending) can
become invalid mid-state:
- Enemy base captured → the "enemy base" no longer belongs to the
  enemy; attacking it is nonsense.
- Own base captured → Defending is over; the match is ending.

On every tick, the behavior module checks whether the current
target is still valid (base still belongs to the other team /
own team respectively). If invalid: force-exit to Spawning on
respawn OR to Transiting with a recomputed target on next tick.
This exit overrides all hysteresis — a captured base is a terminal
condition, no hold-for.

### Path following (Pure Pursuit carrot)

State holds a `path: Vec<Vec2>`. That's it — no generation counter,
no path-target hash. Invalidation is handled by re-computing when
the specific triggers below fire.

**Path computation** (on state entry or when current path is
invalidated):

```rust
// bot_pathfinder.rs::trace_path
pub fn trace_path(
    flow: &FlowField,
    terrain: &Terrain,
    start: Vec2,
    goal: Vec2,
    step_m: f32,        // 80 m — finer than corridor widths in
                        // the sparsened arena to avoid stepping
                        // onto surviving islands between samples
    max_waypoints: usize, // ~25 at step=80 m covers 2000 m
) -> Vec<Vec2> {
    let mut path = vec![start];
    let mut p = start;
    for _ in 0..max_waypoints {
        let Some(dir) = flow.sample(p) else { break };
        let next = p + dir * step_m;
        // Per-waypoint terrain check. If the candidate waypoint
        // lands on land (sparsen survivors, base arms), skip
        // ahead by half-step and re-sample — the flow field
        // itself should have routed around this, but a straight
        // extrapolation across a narrow channel can clip a
        // corner. Bail if we're still stuck after one retry.
        if terrain.sample(next).unwrap_or(Altitude::MIN) >= terrain::SAND_LEVEL {
            let retry = p + dir * (step_m * 0.5);
            if terrain.sample(retry).unwrap_or(Altitude::MIN) >= terrain::SAND_LEVEL {
                break;
            }
            p = retry;
        } else {
            p = next;
        }
        path.push(p);
        if (p - goal).length() < step_m {
            path.push(goal);
            break;
        }
    }
    path
}
```

Step size 80 m is smaller than half the sparsened corridor width
(700 m) and smaller than typical surviving-island diameter — any
mid-step cell-straddling still lands in navigable water most of
the time; the terrain-check-and-retry handles the rest.

**Carrot picking** (per-tick):

```rust
// bot_behavior.rs
fn pick_carrot(
    path: &[Vec2],
    ship_pos: Vec2,
    ship_heading: Vec2,
    ship_speed: f32,
    lookahead_secs: f32,
) -> CarrotResult {
    // 1. Find the closest point on the path to ship_pos.
    //    Iterate path segments, compute perpendicular projection
    //    onto each, track minimum distance + which segment.
    // 2. If that closest-point segment is BEHIND the ship
    //    (i.e. (closest - ship_pos).dot(ship_heading) < 0 AND
    //    distance > 200 m), return CarrotResult::NeedReplan.
    //    A carrot behind the ship would invert direction_target
    //    and cause the exact twirl this plan is eliminating.
    // 3. Otherwise, advance along the path from the closest point
    //    by ship_speed * lookahead_secs. Return the resulting
    //    world point as CarrotResult::Carrot(Vec2).
    // 4. If the advance goes past path's last waypoint, clamp to
    //    path.last() and return CarrotResult::Carrot(last).
}

enum CarrotResult {
    Carrot(Vec2),
    NeedReplan,
}
```

**Steering command** (per-tick):

```rust
// bot_behavior.rs
let target_pos = match pick_carrot(&path, ship_pos, ship_heading,
                                   ship_speed, lookahead_secs) {
    CarrotResult::Carrot(c) => c,
    CarrotResult::NeedReplan => {
        path = trace_path(flow, terrain, ship_pos, target, 80.0, 25);
        *path.get(1).unwrap_or(&target) // next waypoint or goal
    }
};
let direction_target = Angle::from(target_pos - ship_pos);
```

**Local forces don't aggregate into direction_target, they
perturb it.** Renaming the section from the earlier draft's "no
force aggregation" — DHH was right, there IS still a terrain
repel and a torpedo dodge. The load-bearing claim is NOT that
there are no local forces; it is that *local forces cannot flip
the high-level heading*. Concretely:

- **Terrain close-range repel**: 8-sample ring at 1 ship-length
  from ship nose. If land is sampled in any of the 8 rays, rotate
  `direction_target` by UP TO 20° (capped — not summed) away
  from the centroid of land-containing samples. Cap means: even
  if all 8 samples are land, the max rotation is still 20°.
  Cannot flip heading.

- **Torpedo dodge**: one-shot check. If an enemy torpedo is
  within 1.5 ship-lengths on a collision heading (dot product of
  torpedo velocity vs ship-relative position < -0.5), rotate
  `direction_target` perpendicular to the torpedo axis for this
  tick. Tracked via `dodge_cooldown_ticks` (min 10 ticks between
  dodges, keyed by threat entity id) — next tick sees the same
  torpedo but the cooldown prevents continuous re-perturbation.

- **Phase 3b throttle**: composes via `velocity_target_scale`
  (not direction). See §Phase 3b composition below.

Both perturbations have explicit caps that make "flip the heading"
impossible by construction. This is the distinction from boids
aggregation, where summed forces with no caps produce net
direction = angle(sum) and therefore can flip freely.

**Path replanning** (only these three triggers):

- Target changed (e.g. Transiting → Defending, or target-base
  invalidated per §Target invalidation).
- `pick_carrot` returned `NeedReplan` (drifted off-path, or path
  ran past ship's rear).
- Bot's distance to nearest waypoint > 600 m (monitored per-tick).

The original draft's "every 100 ticks as cheap sanity rebuild" is
REMOVED. A valid path should not be rebuilt. Destructible terrain
mid-match was the motivating concern; if it actually proves to
invalidate paths, the behavior will be a reviewable "bots run
into freshly-created land," and THAT bug earns a dedicated trigger
(e.g. "rebuild when terrain mutated within 200 m of any waypoint
this tick") — not a blind timer.

### State transitions — hysteresis table

Committing is NOT a row — it is a boolean mode re-evaluated every
tick (`committing = within(700m, enemy_base) && target == enemy_base`).
No transition, no hysteresis; the predicate's tick-to-tick
stability comes from the 700 m threshold being far from the
arena midpoint.

| From → To | Entry condition | Entry hold-for (ticks) | Exit condition | Exit hold-for (ticks) |
|---|---|---|---|---|
| Spawning → Transiting | status becomes Alive | event, not timed | — | — |
| Transiting → Engaging | `closest_enemy.is_some()` AND within bot's sensor range | 3 | `closest_enemy.is_none()` for 40 ticks (4 s at 10 Hz) | **40** |
| Any non-Defending → Defending | `own_base_capture_progress > 10_000 ms` AND (I am top-3 closest teammate to own base OR <3 teammates alive) | 5 | `own_base_capture_progress < 3_000 ms` for 5 ticks | **5** |
| Defending → Transiting | per Defending exit above | — | — | — |
| * → Spawning | status becomes Dead | event | status becomes Alive | event |

**Threshold derivations:**

- 40-tick Engaging exit (4 s): mk48's passive-sensor visibility
  has known flicker at range that can last 2-3 s during a single
  enemy evasion maneuver. 4 s = 2× worst-case flicker; an enemy
  still absent at that point has actually disengaged. Original
  draft had 20 ticks (2 s) which is within flicker duration and
  would produce the same class of oscillation this plan is
  eliminating.

- 10 s / 3 s Defending: capture takes 30 s per ship, so an
  attacker must be solo in the base for a third of the capture
  to trigger Defending. Leaving for 3 s with capture reset is a
  strong signal the threat is gone. Both thresholds are
  conservative starting points — the `cta_bot_state_transitions`
  counter (see Acceptance) will flag if they flap in practice.

- 5-tick Defending entry hold: 0.5 s to avoid firing Defending on
  a single-tick glitch where capture_progress briefly exceeds
  10 s due to floating-point edge cases.

- 3-tick Transiting → Engaging entry hold: prevents entering
  Engaging on a sensor-ghost single-tick reading.

Transitions INTO a state are easy (short hold or event-driven);
transitions OUT require sustained evidence. That asymmetry is the
hysteresis.

### Force aggregation, replaced with bounded local perturbation

The current `movement = Σ forces` becomes a call into the behavior
module that returns a direction and a velocity scale:

```rust
// bot.rs inner update (new shape)
let behavior_output = bot_behavior::tick(&mut self.behavior_state, &BehaviorInputs {
    ship_pos,
    ship_heading,
    ship_speed,
    ship_hp_percent: health_percent,
    closest_enemy: closest_enemy_info,   // position, distance, id
    closest_torpedo: closest_torpedo_info, // for dodge check
    own_base,
    enemy_base,
    own_base_capture_ms,
    own_base_owner,     // for target-invalidation check
    enemy_base_owner,   // ditto
    teammate_positions: &teammates_sorted_by_dist_to_own_base,
    flow_field,
    terrain,
});
// returns BehaviorOutput { direction_target: Angle, velocity_target_scale: f32 }
```

All terrain/enemy/teammate handling happens INSIDE
`bot_behavior::tick`. The current bot.rs lines 150-326 — the
movement-vector aggregation and all the per-contact spring/repel
logic — go away.

The two local perturbations spelled out in §Path following
(terrain repel ≤ 20° rotation cap, torpedo dodge keyed by cooldown)
live inside the behavior module. Both have explicit caps/cooldowns
that prevent them from flipping direction_target or being applied
repeatedly.

### Phase 3b composition

Phase 3b's `pending_speed_scale` is the turn-budget throttle
(clamps when the upcoming heading change exceeds what the ship's
turn rate can execute at current speed). It is computed by the
OUTER update (bot.rs line ~598 prior), separately from the inner
update. The inner update currently applies it via:

```rust
velocity_target: data.speed * self.pending_speed_scale * difficulty_multiplier,
```

After this plan, velocity_target becomes:

```rust
velocity_target: data.speed * composed_speed_scale * difficulty_multiplier,

where composed_speed_scale =
    if behavior_output.velocity_target_scale == 1.0 /* Committing */ {
        // Committing bypasses the Phase 3b throttle — bots pushing
        // the capture ring should not be slowed down by a turn-
        // budget constraint that fires near base arms. Accept the
        // risk of occasional terrain kiss in the final 700 m.
        1.0
    } else {
        behavior_output.velocity_target_scale.min(self.pending_speed_scale)
    };
```

`min(behavior, throttle)` means the TIGHTER of the two limits
applies, except in Committing where the throttle is bypassed.
The bypass is explicit and scoped to the final push.

## What this does NOT do

- Does NOT change aim/firing logic (bot.rs:328–431). Those stay.
- Does NOT change submarine submerge hysteresis. Stays.
- Does NOT add influence maps or tactical-safety pathing. If the
  state-machine+path alone isn't enough, THAT is the next plan,
  not this one.
- Does NOT modify flow field construction or the sparsen pass
  (Phase 4). They continue to produce the terrain routing layer
  this plan consumes.
- Does NOT modify the match state machine, scoring, base capture,
  or team assignment.
- Does NOT touch the client. Server-only change.

## Acceptance

All existing Phase 3b/4 instrumentation stays. Adds one more counter:

- `cta_bot_state_transitions: u32` — total number of state
  transitions across all bots per match. Sanity-check that bots
  aren't flapping (expect ~50–150 per match; > 500 means a
  transition condition is still flap-prone).

**Pass criteria (all must hold across 3 consecutive matches):**

1. **`enemy_base_reached ≥ 3 bots per team per match`** (was 0
   across Phase 3b, 4, and 5). NON-NEGOTIABLE PRIMARY GATE. The
   architectural change is worth nothing if bots still don't
   arrive.

2. **`≥ 2 base captures across the 3 matches`** (was 0 across all
   prior phases).

3. **`terrain_deaths ≤ 20 per match`**. More permissive than
   Phase 4's ≤ 10 because committed bots pushing hard will
   occasionally crash — that's fine. Hard ceiling is 30.

4. **`cta_bot_state_transitions ≤ 200 per match`**. Expect
   50-150 per match based on estimated state lifetimes (bots
   spend 20-40 s per Transiting/Engaging cycle, with occasional
   Defending swings). Gate at 200 (not 300) so the
   soft-expectation and hard-fail don't have a yawning gap.
   If transitions > 200, a transition condition is still flapping
   and we iterate ONE numeric threshold before giving up — this
   is the one tuning knob allowed in this plan, justified because
   the counter directly measures the symptom.

5. **User observation: "no twirling."** Phase 3b/4/5 failed this
   qualitatively. If EDI-style oscillation persists, the state
   machine isn't achieving its commitment promise.

**Hard stop:** if **`enemy_base_reached`** is still 0 after this
plan ships, the problem is not in the AI architecture — it is
elsewhere (ship physics fundamentally hostile at current map
scale, flow field producing wrong paths, or a bug in the
implementation). At that point the honest options are:

- Revert the whole CTA bot stack to direct-to-goal and accept
  dumb-but-predictable bots (like 1995 RTS hard AI).
- Replace bots with scripted patrol patterns. No tactics, just
  predictable motion along fixed routes.
- Accept that the bots can't reach the enemy base and redesign
  CTA so the human player, not bots, is the primary attacker.

None of those are "tune one more number."

## Files

| File | Change |
|------|--------|
| `server/src/bot_behavior.rs` | NEW — state machine, path follower, transition logic, local-perturbation logic |
| `server/src/bot_pathfinder.rs` | `trace_path` function (uses flow field + terrain together) + unit tests |
| `server/src/bot.rs` | Remove movement aggregation (~lines 150–326); delegate to `bot_behavior::tick`; keep aim/firing (lines 328–431) and submerge (lines 433–447) untouched |
| `server/src/main.rs` | `mod bot_behavior;` |
| `server/src/server.rs` | `cta_bot_state_transitions` counter + match-end log |

One commit. Single cohesive change — the state machine and the
path follower and the bot.rs refactor are all mutually dependent;
splitting them produces a broken intermediate.

## Implementation sequence

1. **`bot_pathfinder::trace_path`** + unit tests. Tests: straight-
   line path on blank terrain; path that curves around an island;
   path cut short at goal; path with unreachable goal (expect
   truncated path, not panic); determinism (same inputs, same
   path). Unit tests need a blank-terrain `Terrain` fixture —
   already exists in `bot_pathfinder::tests::blank_terrain()`; no
   new mock required.

2. **`bot_behavior::State` enum** (`Spawning`,
   `Transiting { committing: bool }`, `Engaging`, `Defending`) +
   **`BehaviorState` struct** (holds `state`, `path`,
   `state_entered_tick`, `ticks_enemy_out_of_range` —
   renamed from the first draft's vaguer
   `ticks_since_enemy_visible`, `defending_exit_ticks`,
   `dodge_cooldown: HashMap<EntityId, u32>`). No
   `path_target_generation` field — dead weight.

3. **Transition predicates as pure functions** with signature:

   ```rust
   fn transition(current: &BehaviorState, inputs: &BehaviorInputs)
       -> Option<State>
   ```

   Returns `Some(new_state)` when a transition condition is met,
   `None` to stay. The caller (`tick`) then updates mutable
   counters and path. Unit test each transition's hysteresis
   independently: "Engaging exits only after 40 ticks with
   `closest_enemy.is_none()`"; "Defending enters only after 10 s
   capture progress with top-3 closeness OR <3 teammates alive";
   "Committing mode flag flips at 700 m boundary crossing."

4. **`pick_carrot`** + unit tests. Tests: straight path (carrot
   ahead of ship); curved path (carrot on the bend); ship far off
   path with closest-point ahead (carrot still produced); ship
   far off path with closest-point behind (returns
   `NeedReplan`); carrot past end of path (clamps to last
   waypoint).

5. **`bot_behavior::tick(inputs) -> BehaviorOutput`** wiring
   states to carrot to local perturbations to output. Unit-test
   each state's output shape with a fixed-input fixture. Test the
   Committing bypass: when `committing && closest_enemy` pulls
   throttle to 0.3, the composed speed scale stays at 1.0.

6. **`bot.rs` refactor**: replace lines 150–326 with the
   `bot_behavior::tick(...)` call. Preserve lines 328–431
   (aim/firing) and lines 433–447 (submerge) unchanged. Update
   the inner-update `velocity_target` expression to use the
   composed speed scale per §Phase 3b composition.

7. **`cta_bot_state_transitions` counter** on Server + match-end
   log line. Follow the Phase 3b counter pattern
   (`world.rs::cta_bot_terrain_deaths` is the closest template
   — increment where transitions happen, log aggregate on
   `MatchEvent::MatchEnded`).

8. **`cargo build && cargo test`**. Expected test impact: none
   of the existing 35 tests touch bot.rs's movement aggregation,
   so no pre-existing test should break. New tests from steps
   1, 3, 4, 5 should all pass. If anything else breaks, fix it.

9. **Commit + restart server + play 3 matches.** Report the gate
   outcome. If primary gate (`enemy_base_reached ≥ 3/team`)
   passes, Phase 6 shipped. If not, hard-stop options take over.

## Dependency on prior phases

This plan ASSUMES Phase 3a (flow field inflation + goal clear)
and Phase 4 (arena expand + sparsen) are in place — the path
follower reads the flow field produced by those phases. Phase 3b
(steering throttle) stays in place and operates on the
`velocity_target_scale` side; untouched by this plan.

## References

### Prior plans (read these for context)
- `plans/cta-arena-expand-and-sparsen.md` (Phase 4, shipped)
- `plans/non-holonomic-ship-steering.md` (Phase 3b, shipped)
- `plans/bot-objective-commitment.md` (Phase 5, reverted)

### Code
- `server/src/bot.rs:150-326` — the movement aggregation being
  replaced
- `server/src/bot.rs:328-431` — aim/firing, preserved
- `server/src/bot.rs:433-447` — submerge hysteresis (reference
  pattern for state-hysteresis elsewhere)
- `server/src/bot_pathfinder.rs:230` — `FlowField::sample` API
  (read by `trace_path`)
- `server/src/match_state.rs:40` — `ArenaLayout::DEFAULT`
  (base positions used for targeting)

### External
- Coulter, R. C. (1992). *Implementation of the Pure Pursuit
  Path Tracking Algorithm.* Carnegie Mellon. Load-bearing: the
  carrot-on-path geometry in §Path following is Pure Pursuit.
- Reynolds, C. W. (1987). *Flocks, Herds, and Schools: A
  Distributed Behavioral Model.* SIGGRAPH. For the record: this
  is what bot.rs:150-326 currently implements, and what this
  plan replaces.

## Decision framing

This is the architectural change the prior five plans danced
around. Each of them took "fix the behavior without restructuring
the decision logic" as a constraint; each failed. The hard-stop in
the Phase 5 plan already told us "this theory is wrong" without
telling us which theory was right — only that weight tuning wasn't
it. State + path commitment is the theory that fits the evidence:
direction jitter, flap, park-at-orbit-range, no multi-step
reasoning are all direct consequences of "no memory, no plan."

If this ships and the primary gate (`enemy_base_reached ≥ 3/team`)
passes, we finally have bots that commit to an objective instead
of integrating a force field. If it fails, the diagnosis narrows
sharply: the problem is no longer "movement decision logic" —
it's either physics at this map scale (ships genuinely can't
execute the turns CTA requires), flow-field routing (wrong paths
even if followed correctly), or implementation (bug in the
behavior module). Each of those is a different Phase 7.
