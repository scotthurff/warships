// SPDX-FileCopyrightText: 2026 Scott Hurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CTA bot behavior — state machine + Pure Pursuit path follower.
//!
//! Replaces the per-tick movement-vector aggregation that lived in
//! `bot.rs` (Reynolds 1987 boids). Each bot now holds explicit
//! state (`Spawning`, `Transiting { committing }`, `Engaging`,
//! `Defending`), a planned path (`Vec<Vec2>` of waypoints in world
//! space), and hysteretic counters for state transitions. Each
//! tick, the bot:
//!
//! 1. Evaluates state transition predicates, applies at most one.
//! 2. Computes a carrot point on the path at a velocity-adaptive
//!    look-ahead distance (Pure Pursuit — Coulter 1992).
//! 3. Applies bounded local perturbations (terrain repel ≤ 20°,
//!    torpedo dodge with cooldown) to the carrot direction.
//! 4. Returns a `BehaviorOutput { direction_target, velocity_scale }`.
//!
//! Path planning (`trace_path`) happens in the OUTER update
//! (server.rs), where the FlowField is accessible, and the result
//! is written into `BehaviorState::path`. The inner update here
//! consumes the path.
//!
//! See plans/bot-state-machine-and-path-following.md.

use crate::bot_pathfinder::FlowField;
use common::altitude::Altitude;
use common::angle::Angle;
use common::terrain::{Terrain, SAND_LEVEL};
use kodiak_server::glam::Vec2;
use std::collections::HashMap;

// ─── Public types ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    /// Dead or spawning — no control output. Exits on alive.
    Spawning,
    /// Moving toward target along `path`. `committing` flips on
    /// when the bot is within 700 m of the enemy base (and still
    /// targeting it) — a mode, not a distinct state.
    Transiting { committing: bool },
    /// Close enough to an enemy to shoot. Same path as Transiting
    /// but with a shorter carrot look-ahead so the ship can curve
    /// around the target without losing path progress.
    Engaging,
    /// Own base under sustained attack — stay at own base and
    /// shoot attackers. Path target = own base center.
    Defending,
}

#[derive(Debug)]
pub struct BehaviorState {
    pub state: State,
    /// World-space waypoints from current position to target.
    /// Recomputed by the outer update on state entry or when
    /// invalidated (`pick_carrot` returned `NeedReplan`, drift
    /// > 600 m, target change).
    pub path: Vec<Vec2>,
    /// Target the current `path` was planned toward. Used to
    /// detect "target swap" and trigger a replan.
    pub path_target: Vec2,
    /// Ticks during which `closest_enemy.is_none()` has been
    /// continuously true. Used for Engaging → Transiting exit
    /// hysteresis (40-tick hold-out).
    pub ticks_enemy_out: u32,
    /// Ticks during which own-base capture progress has been
    /// continuously < 3s. Used for Defending exit hysteresis.
    pub ticks_defense_exit: u32,
    /// Ticks during which transition predicates have continuously
    /// indicated Engaging entry. Prevents single-tick sensor
    /// ghosts from entering Engaging.
    pub ticks_engage_pending: u32,
    /// Similar counter for Defending entry.
    pub ticks_defend_pending: u32,
    /// Per-threat dodge cooldown. Map from threat entity id to
    /// remaining cooldown ticks. Prevents continuous re-dodge
    /// of the same torpedo.
    pub dodge_cooldown: HashMap<u32, u32>,
    /// Increments every state transition for acceptance
    /// instrumentation. Reset on Spawning re-entry.
    pub transitions_this_life: u32,
    /// Team-context inputs written by the OUTER update each tick
    /// (where `server` / `match_state` are accessible). The INNER
    /// update reads these into `BehaviorInputs` alongside its own
    /// contact scan. None = outer hasn't run yet for this bot.
    pub pending_outer: Option<PendingOuterInputs>,
}

#[derive(Debug, Clone)]
pub struct PendingOuterInputs {
    pub own_base: Vec2,
    pub enemy_base: Vec2,
    pub own_base_capture_ms: u128,
    pub is_top_3_defender: bool,
}

impl Default for BehaviorState {
    fn default() -> Self {
        Self {
            state: State::Spawning,
            path: Vec::new(),
            path_target: Vec2::ZERO,
            ticks_enemy_out: 0,
            ticks_defense_exit: 0,
            ticks_engage_pending: 0,
            ticks_defend_pending: 0,
            dodge_cooldown: HashMap::new(),
            transitions_this_life: 0,
            pending_outer: None,
        }
    }
}

/// Per-tick inputs the inner update assembles from the bot's local
/// view (contacts, ship state) plus the outer-update-supplied
/// path/target info.
pub struct BehaviorInputs<'a> {
    pub ship_pos: Vec2,
    pub ship_heading: Vec2, // unit vector
    pub ship_speed: f32,    // m/s
    pub ship_length: f32,

    /// World-space position of closest sensed enemy boat, if any.
    /// `None` when no enemy is in the bot's sensor range. Drives
    /// Engaging state transitions.
    pub closest_enemy: Option<Vec2>,

    /// World-space position of the nearest teammate boat within
    /// 2× ship-length, if any. Used by the teammate-separation
    /// perturbation — bots pathing the same flow field converge
    /// on identical trajectories and collide without this.
    pub nearest_teammate: Option<Vec2>,

    /// Torpedo (or equivalent weapon) on a collision heading
    /// within 1.5 ship-lengths, if any. Caller is responsible for
    /// the dot-product heuristic; this is just the threat info.
    pub incoming_torpedo: Option<TorpedoThreat>,

    /// Current team's target info from the outer update.
    pub own_base: Vec2,
    pub enemy_base: Vec2,
    /// Capture progress (ms) of the bot's OWN base by enemies.
    pub own_base_capture_ms: u128,
    /// Whether this bot is one of the top-3 closest teammates to
    /// its own base (or, if < 3 teammates alive, any surviving
    /// teammate — caller computes the short-circuit).
    pub is_top_3_defender: bool,

    /// "Rusher" trait — when true, this bot ignores nearby enemies
    /// while within the final-push radius of the enemy base and
    /// drives for the capture ring. Roughly 35% of bots are
    /// rushers; the rest engage in combat as usual. Mixed-team
    /// behavior means some bots push objectives while others
    /// skirmish at midfield.
    pub prefers_rush: bool,

    /// Terrain sample source for the close-range land repel.
    pub terrain: &'a Terrain,
    /// World border. Ships must stay inside this radius.
    pub world_radius: f32,
}

pub struct TorpedoThreat {
    pub id: u32,
    pub relative_pos: Vec2, // ship-relative position of torpedo
}

pub struct BehaviorOutput {
    pub direction_target: Angle,
    /// 0.3..1.0. Composed with Phase 3b's `pending_speed_scale` at
    /// the bot.rs callsite: `min(behavior.scale, pending.scale)`,
    /// except when `Transiting { committing: true }` bypasses the
    /// throttle (returns 1.0, and the caller should NOT compose).
    pub velocity_scale: f32,
    /// True when the caller should bypass Phase 3b's throttle —
    /// only set during the Committing final push.
    pub bypass_phase3b_throttle: bool,
}

// ─── Tick entry point ───────────────────────────────────────────

/// Main per-tick entry. Mutates `state` (transition counters, state
/// changes, dodge cooldowns, transition count). Pure with respect
/// to `inputs`.
///
/// Path planning happens OUTSIDE this function — see server.rs
/// outer update. If `state.path` is empty or stale, the output
/// direction falls back to "ship heading + toward target" so the
/// ship doesn't twirl even before the outer gets a chance to plan.
pub fn tick(state: &mut BehaviorState, inputs: &BehaviorInputs) -> BehaviorOutput {
    // 1. Tick dodge cooldowns down.
    state.dodge_cooldown.retain(|_, ticks| {
        *ticks = ticks.saturating_sub(1);
        *ticks > 0
    });

    // 2. Update hysteresis counters FIRST based on current inputs.
    //    Done before `next_state` so the transition predicate sees
    //    the counter value for THIS tick, not last tick. Produces
    //    the "exit after N ticks of no enemy" semantics expected by
    //    the hysteresis table (N ticks of condition = transition on
    //    the Nth tick, not the N+1th).
    update_hysteresis_counters(state, inputs);

    // 3. Evaluate transitions. Apply at most one per tick.
    if let Some(new_state) = next_state(state, inputs) {
        state.state = new_state;
        state.transitions_this_life = state.transitions_this_life.saturating_add(1);
        // Reset per-state entry counters on transition.
        state.ticks_enemy_out = 0;
        state.ticks_engage_pending = 0;
        state.ticks_defense_exit = 0;
        state.ticks_defend_pending = 0;
    }

    // 4. Update Committing mode flag (re-evaluated every tick; it's
    //    not a state transition).
    if let State::Transiting { committing } = state.state {
        let should_commit = state.path_target == inputs.enemy_base
            && (inputs.ship_pos - inputs.enemy_base).length() < COMMIT_RADIUS;
        if should_commit != committing {
            state.state = State::Transiting { committing: should_commit };
            // Committing flip is NOT counted as a transition for
            // the flap guard — it's a predicate re-evaluation.
        }
    }

    // 5. Compute direction + velocity for the current state.
    compute_output(state, inputs)
}

// ─── Tunables ────────────────────────────────────────────────────

/// Distance from enemy base at which Committing mode flips on.
const COMMIT_RADIUS: f32 = 700.0;

/// Ticks of `closest_enemy.is_none()` required to exit Engaging.
/// 40 = 4 s at 10 Hz. mk48's passive-sensor visibility can flicker
/// for 2-3 s at range during a single enemy evasion, so 20 ticks
/// (2 s — the first-draft value) would re-trigger on the same
/// enemy. 40 ticks beats that by 2x.
const ENGAGE_EXIT_TICKS: u32 = 40;

/// Ticks of sustained enemy presence required to enter Engaging.
/// 3 ticks = 300 ms guards against a single-tick sensor ghost.
const ENGAGE_ENTER_TICKS: u32 = 3;

/// Capture progress threshold (ms) to ENTER Defending. A full
/// capture is 30 s; 10 s = 1/3 of capture, significant but not
/// panic-level.
const DEFEND_ENTER_CAPTURE_MS: u128 = 10_000;

/// Capture progress threshold (ms) to EXIT Defending.
const DEFEND_EXIT_CAPTURE_MS: u128 = 3_000;

/// Ticks of sustained `capture_ms > enter` required to enter
/// Defending. 5 ticks = 500 ms; guards against float-edge flickers.
const DEFEND_ENTER_TICKS: u32 = 5;

/// Ticks of sustained `capture_ms < exit` required to exit
/// Defending.
const DEFEND_EXIT_TICKS: u32 = 5;

/// Path look-ahead in seconds. Carrot is placed at
/// `ship_pos + ship_speed * lookahead_secs` along the path.
const LOOKAHEAD_SECS_TRANSITING: f32 = 2.5;
const LOOKAHEAD_SECS_ENGAGING: f32 = 1.25; // 0.5× Transiting
const LOOKAHEAD_SECS_COMMITTING: f32 = 5.0; // 2× Transiting

/// Max rotation applied by the close-range terrain repel.
/// Cap is the point — land within 1 ship-length can nudge the
/// heading up to ±20°, but cannot flip it.
const TERRAIN_REPEL_CAP_DEGREES: f32 = 20.0;

/// Cooldown (ticks) between dodges of the same torpedo. Prevents
/// per-tick re-perturbation.
const DODGE_COOLDOWN_TICKS: u32 = 10;

/// Velocity scale floor for Defending + Engaging. Below this the
/// ship can't accelerate fast enough to turn. Transiting uses 1.0
/// (no floor — the Phase 3b throttle handles that side).
const ENGAGE_VELOCITY_FLOOR: f32 = 0.6;
const DEFEND_VELOCITY_FLOOR: f32 = 0.5;

// ─── State transitions ──────────────────────────────────────────

/// Returns `Some(new_state)` if a transition should fire this tick,
/// `None` to stay in the current state. At most one transition
/// per tick. Pure function.
fn next_state(state: &BehaviorState, inputs: &BehaviorInputs) -> Option<State> {
    // Event-driven: Spawning → Transiting on alive (caller detects
    // Alive status; tick() is only called when alive, so if we're
    // in Spawning when tick() runs, enter Transiting).
    if matches!(state.state, State::Spawning) {
        return Some(State::Transiting { committing: false });
    }

    let own_base_under_attack =
        inputs.own_base_capture_ms > DEFEND_ENTER_CAPTURE_MS && inputs.is_top_3_defender;
    let own_base_safe = inputs.own_base_capture_ms < DEFEND_EXIT_CAPTURE_MS;

    match state.state {
        State::Spawning => unreachable!("handled above"),

        State::Transiting { .. } | State::Engaging => {
            // Priority: Defending overrides everything.
            if own_base_under_attack
                && state.ticks_defend_pending >= DEFEND_ENTER_TICKS
            {
                return Some(State::Defending);
            }

            // Rusher override — when within the final-push radius
            // of the enemy base AND this bot is flagged as a
            // rusher, ignore nearby enemies and stay in the
            // committing-Transiting push. The other ~65% of bots
            // behave normally (enter Engaging on enemy visibility).
            let in_commit_range = (inputs.ship_pos - inputs.enemy_base).length()
                < COMMIT_RADIUS;
            let rusher_pushing = inputs.prefers_rush && in_commit_range;

            // Engaging ↔ Transiting.
            if matches!(state.state, State::Engaging) {
                // Rusher that's entered the commit radius exits
                // Engaging immediately (no hold-out) — there's a
                // capture to finish.
                if rusher_pushing {
                    return Some(State::Transiting { committing: true });
                }
                if inputs.closest_enemy.is_none()
                    && state.ticks_enemy_out >= ENGAGE_EXIT_TICKS
                {
                    return Some(State::Transiting { committing: false });
                }
            } else if !rusher_pushing
                && inputs.closest_enemy.is_some()
                && state.ticks_engage_pending >= ENGAGE_ENTER_TICKS
            {
                return Some(State::Engaging);
            }
            None
        }

        State::Defending => {
            if own_base_safe && state.ticks_defense_exit >= DEFEND_EXIT_TICKS {
                return Some(State::Transiting { committing: false });
            }
            None
        }
    }
}

fn update_hysteresis_counters(state: &mut BehaviorState, inputs: &BehaviorInputs) {
    // Engaging exit counter.
    if inputs.closest_enemy.is_none() {
        state.ticks_enemy_out = state.ticks_enemy_out.saturating_add(1);
    } else {
        state.ticks_enemy_out = 0;
    }

    // Engaging entry counter.
    if inputs.closest_enemy.is_some() {
        state.ticks_engage_pending = state.ticks_engage_pending.saturating_add(1);
    } else {
        state.ticks_engage_pending = 0;
    }

    // Defending entry counter.
    if inputs.own_base_capture_ms > DEFEND_ENTER_CAPTURE_MS && inputs.is_top_3_defender {
        state.ticks_defend_pending = state.ticks_defend_pending.saturating_add(1);
    } else {
        state.ticks_defend_pending = 0;
    }

    // Defending exit counter.
    if inputs.own_base_capture_ms < DEFEND_EXIT_CAPTURE_MS {
        state.ticks_defense_exit = state.ticks_defense_exit.saturating_add(1);
    } else {
        state.ticks_defense_exit = 0;
    }
}

// ─── Carrot + output ────────────────────────────────────────────

fn compute_output(state: &mut BehaviorState, inputs: &BehaviorInputs) -> BehaviorOutput {
    let (lookahead_secs, velocity_scale, bypass_throttle) = match state.state {
        State::Spawning => {
            // Shouldn't happen — tick() transitions out of Spawning
            // immediately. But return a safe default.
            return BehaviorOutput {
                direction_target: Angle::from(inputs.ship_heading),
                velocity_scale: 1.0,
                bypass_phase3b_throttle: false,
            };
        }
        State::Transiting { committing: true } => {
            (LOOKAHEAD_SECS_COMMITTING, 1.0, true)
        }
        State::Transiting { committing: false } => {
            (LOOKAHEAD_SECS_TRANSITING, 1.0, false)
        }
        State::Engaging => (LOOKAHEAD_SECS_ENGAGING, ENGAGE_VELOCITY_FLOOR, false),
        State::Defending => (LOOKAHEAD_SECS_TRANSITING, DEFEND_VELOCITY_FLOOR, false),
    };

    // If path is empty or stale, fall back to direct-to-target
    // direction. This is a safe default; the outer update should
    // populate the path within a tick or two.
    let target = match state.state {
        State::Defending => inputs.own_base,
        _ => {
            // Attacker: heading toward enemy base (or whatever the
            // outer update's path was planned for — matches
            // path_target).
            if state.path_target == Vec2::ZERO {
                inputs.enemy_base
            } else {
                state.path_target
            }
        }
    };

    let carrot = pick_carrot(
        &state.path,
        inputs.ship_pos,
        inputs.ship_heading,
        inputs.ship_speed,
        lookahead_secs,
    )
    .unwrap_or_else(|| {
        // Path empty or ship far off path with closest-point
        // behind → fall back to direct-to-target.
        target
    });

    // Base direction: carrot or target.
    let mut direction = carrot - inputs.ship_pos;
    if direction.length_squared() < 1.0 {
        // At the target — keep current heading.
        direction = inputs.ship_heading;
    }
    let mut direction_angle = Angle::from(direction);

    // Bounded local perturbation: terrain repel (capped ≤20°).
    direction_angle = apply_terrain_repel(direction_angle, inputs);

    // Teammate separation DISABLED. The first version used a range
    // of 2× ship-length which scales badly — at Iowa's 270 m length,
    // that's a 540 m dodge zone triggering constant heading
    // perturbation whenever ANY teammate is within half a kilometer.
    // Ships swerved off-path into surviving sparsen islands,
    // regressing terrain_deaths from 37 to 67 in live measurement.
    // Keeping the `apply_teammate_separation` function defined for
    // future reuse with fixed-distance (not length-scaled) range;
    // the call is removed. Bots may bump into each other
    // occasionally — that's a less-bad failure mode than swerving
    // into land.
    // (Previously: direction_angle = apply_teammate_separation(...))

    // Torpedo dodge (one-shot, cooldown-gated).
    direction_angle = apply_torpedo_dodge_if_needed(direction_angle, inputs, state);

    BehaviorOutput {
        direction_target: direction_angle,
        velocity_scale,
        bypass_phase3b_throttle: bypass_throttle,
    }
}

/// Result of `pick_carrot`. `None` signals that the ship has
/// drifted past the path (closest-point is behind heading) and
/// the caller should treat it as a replan trigger. The outer
/// update will observe the empty/stale path and rebuild it on
/// the next tick.
pub fn pick_carrot(
    path: &[Vec2],
    ship_pos: Vec2,
    ship_heading: Vec2,
    ship_speed: f32,
    lookahead_secs: f32,
) -> Option<Vec2> {
    if path.is_empty() {
        return None;
    }
    if path.len() == 1 {
        return Some(path[0]);
    }

    // Find closest segment to ship_pos.
    let mut best: Option<(usize, f32, Vec2)> = None; // (segment_idx, t, closest)
    for i in 0..path.len() - 1 {
        let a = path[i];
        let b = path[i + 1];
        let ab = b - a;
        let denom = ab.length_squared();
        let t = if denom > 0.0 {
            ((ship_pos - a).dot(ab) / denom).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let closest = a + ab * t;
        let dist_sq = (ship_pos - closest).length_squared();
        if best.as_ref().map(|b| dist_sq < b.1).unwrap_or(true) {
            best = Some((i, dist_sq, closest));
        }
    }
    let (seg_idx, _dist_sq, closest) = best?;

    // Replan if the closest point is significantly behind the ship.
    let to_closest = closest - ship_pos;
    if to_closest.length() > 200.0 && ship_heading.dot(to_closest) < 0.0 {
        return None;
    }

    // Advance `lookahead_dist` meters along the path from `closest`.
    // Minimum 20 m keeps the carrot ahead of the ship even when
    // stationary (e.g. right after spawn). No large floor — the
    // carrot-on-next-waypoint behavior is desirable.
    let lookahead_dist = (ship_speed * lookahead_secs).max(20.0);
    let mut remaining = lookahead_dist;
    let mut point = closest;
    let mut i = seg_idx;
    while i < path.len() - 1 {
        let seg_end = path[i + 1];
        let seg_len = (seg_end - point).length();
        if seg_len >= remaining {
            let dir = (seg_end - point).normalize_or_zero();
            return Some(point + dir * remaining);
        }
        remaining -= seg_len;
        point = seg_end;
        i += 1;
    }
    // Past end of path.
    Some(*path.last().unwrap())
}

fn apply_terrain_repel(direction: Angle, inputs: &BehaviorInputs) -> Angle {
    const SAMPLES: u32 = 8;
    let probe_dist = inputs.ship_length;
    let ship_nose = inputs.ship_pos + inputs.ship_heading * probe_dist * 0.5;

    let mut land_centroid = Vec2::ZERO;
    let mut count = 0;
    for i in 0..SAMPLES {
        let angle =
            Angle::from_radians(i as f32 * (2.0 * std::f32::consts::PI / SAMPLES as f32));
        let sample = ship_nose + angle.to_vec() * probe_dist;
        let alt = inputs.terrain.sample(sample).unwrap_or(Altitude::MIN);
        let in_border = sample.length_squared() > inputs.world_radius.powi(2);
        if alt >= SAND_LEVEL || in_border {
            land_centroid += sample - inputs.ship_pos;
            count += 1;
        }
    }
    if count == 0 {
        return direction;
    }
    // Rotate direction away from land_centroid by UP TO cap degrees.
    let land_dir = (land_centroid / count as f32).normalize_or_zero();
    if land_dir.length_squared() < 0.1 {
        return direction;
    }
    let current_vec = direction.to_vec();
    let cross_sign = current_vec.x * land_dir.y - current_vec.y * land_dir.x;
    // Rotate opposite sign (away from land) by cap.
    let cap_rad = TERRAIN_REPEL_CAP_DEGREES.to_radians();
    let rotation = if cross_sign > 0.0 { -cap_rad } else { cap_rad };
    direction + Angle::from_radians(rotation)
}

fn apply_teammate_separation(direction: Angle, inputs: &BehaviorInputs) -> Angle {
    let Some(mate) = inputs.nearest_teammate else {
        return direction;
    };
    let to_mate = mate - inputs.ship_pos;
    let dist = to_mate.length();
    if dist < 0.1 || dist > inputs.ship_length * 2.0 {
        return direction;
    }
    // Rotate `direction` away from the teammate by up to 15°, with
    // magnitude proportional to closeness (full 15° at touching,
    // 0° at 2× ship-length).
    let closeness = 1.0 - (dist / (inputs.ship_length * 2.0)).clamp(0.0, 1.0);
    let cap_rad = 15f32.to_radians() * closeness;
    let mate_dir = to_mate.normalize_or_zero();
    let current_vec = direction.to_vec();
    let cross_sign = current_vec.x * mate_dir.y - current_vec.y * mate_dir.x;
    let rotation = if cross_sign > 0.0 { -cap_rad } else { cap_rad };
    direction + Angle::from_radians(rotation)
}

fn apply_torpedo_dodge_if_needed(
    direction: Angle,
    inputs: &BehaviorInputs,
    state: &mut BehaviorState,
) -> Angle {
    let Some(threat) = &inputs.incoming_torpedo else {
        return direction;
    };
    // Check cooldown first.
    if state.dodge_cooldown.contains_key(&threat.id) {
        return direction;
    }
    // Perpendicular dodge: rotate 60° away from the torpedo axis.
    let torpedo_axis = threat.relative_pos.normalize_or_zero();
    if torpedo_axis.length_squared() < 0.1 {
        return direction;
    }
    let current_vec = direction.to_vec();
    let cross_sign = current_vec.x * torpedo_axis.y - current_vec.y * torpedo_axis.x;
    let dodge_rad = 60f32.to_radians();
    let rotation = if cross_sign > 0.0 { dodge_rad } else { -dodge_rad };
    state.dodge_cooldown.insert(threat.id, DODGE_COOLDOWN_TICKS);
    direction + Angle::from_radians(rotation)
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_carrot_straight_path_returns_point_ahead() {
        let path = vec![Vec2::new(0.0, 0.0), Vec2::new(1000.0, 0.0)];
        let ship_pos = Vec2::new(100.0, 0.0);
        let ship_heading = Vec2::new(1.0, 0.0);
        let carrot = pick_carrot(&path, ship_pos, ship_heading, 10.0, 5.0).unwrap();
        // Lookahead 10 * 5 = 50 m. Closest point is (100, 0).
        // Carrot should be at (150, 0).
        assert!((carrot - Vec2::new(150.0, 0.0)).length() < 1.0);
    }

    #[test]
    fn pick_carrot_returns_last_when_past_end() {
        let path = vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        let ship_pos = Vec2::new(90.0, 0.0);
        let ship_heading = Vec2::new(1.0, 0.0);
        // Lookahead 500 m >> path length.
        let carrot = pick_carrot(&path, ship_pos, ship_heading, 10.0, 50.0).unwrap();
        assert_eq!(carrot, Vec2::new(100.0, 0.0));
    }

    #[test]
    fn pick_carrot_behind_ship_returns_none() {
        let path = vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
        // Ship is 500 m PAST the end, heading away.
        let ship_pos = Vec2::new(600.0, 0.0);
        let ship_heading = Vec2::new(1.0, 0.0);
        let carrot = pick_carrot(&path, ship_pos, ship_heading, 10.0, 5.0);
        assert!(carrot.is_none(), "expected NeedReplan, got {:?}", carrot);
    }

    #[test]
    fn pick_carrot_empty_path_returns_none() {
        let path: Vec<Vec2> = vec![];
        let carrot = pick_carrot(&path, Vec2::ZERO, Vec2::X, 10.0, 5.0);
        assert!(carrot.is_none());
    }

    #[test]
    fn state_spawning_transitions_to_transiting_on_tick() {
        let mut state = BehaviorState::default();
        assert_eq!(state.state, State::Spawning);
        let terrain = common::terrain::Terrain::new();
        let inputs = BehaviorInputs {
            ship_pos: Vec2::ZERO,
            ship_heading: Vec2::X,
            ship_speed: 0.0,
            ship_length: 30.0,
            closest_enemy: None,
            nearest_teammate: None,
            incoming_torpedo: None,
            own_base: Vec2::new(0.0, 1000.0),
            enemy_base: Vec2::new(0.0, -1000.0),
            own_base_capture_ms: 0,
            is_top_3_defender: false,
            prefers_rush: false,
            terrain: &terrain,
            world_radius: 3000.0,
        };
        let _ = tick(&mut state, &inputs);
        assert!(matches!(state.state, State::Transiting { .. }));
    }

    #[test]
    fn engaging_exit_requires_full_hold_out() {
        let mut state = BehaviorState {
            state: State::Engaging,
            ..Default::default()
        };
        let terrain = common::terrain::Terrain::new();
        let mut inputs = BehaviorInputs {
            ship_pos: Vec2::ZERO,
            ship_heading: Vec2::X,
            ship_speed: 10.0,
            ship_length: 30.0,
            closest_enemy: None, // enemy gone
            nearest_teammate: None,
            incoming_torpedo: None,
            own_base: Vec2::new(0.0, 1000.0),
            enemy_base: Vec2::new(0.0, -1000.0),
            own_base_capture_ms: 0,
            is_top_3_defender: false,
            prefers_rush: false,
            terrain: &terrain,
            world_radius: 3000.0,
        };
        // Tick ENGAGE_EXIT_TICKS - 1 times; should still be Engaging.
        for _ in 0..(ENGAGE_EXIT_TICKS - 1) {
            let _ = tick(&mut state, &inputs);
        }
        assert_eq!(state.state, State::Engaging);
        // One more tick triggers the exit.
        let _ = tick(&mut state, &inputs);
        assert!(matches!(state.state, State::Transiting { .. }));
        let _ = inputs.closest_enemy; // silence unused-mut
    }

    #[test]
    fn defending_enters_only_with_sustained_capture_and_top3() {
        let mut state = BehaviorState {
            state: State::Transiting { committing: false },
            ..Default::default()
        };
        let terrain = common::terrain::Terrain::new();
        let inputs_not_top3 = BehaviorInputs {
            ship_pos: Vec2::ZERO,
            ship_heading: Vec2::X,
            ship_speed: 10.0,
            ship_length: 30.0,
            closest_enemy: None,
            nearest_teammate: None,
            incoming_torpedo: None,
            own_base: Vec2::new(0.0, 1000.0),
            enemy_base: Vec2::new(0.0, -1000.0),
            own_base_capture_ms: 15_000, // well over threshold
            is_top_3_defender: false,
            prefers_rush: false,    // but not a defender
            terrain: &terrain,
            world_radius: 3000.0,
        };
        for _ in 0..20 {
            let _ = tick(&mut state, &inputs_not_top3);
        }
        assert!(
            matches!(state.state, State::Transiting { .. }),
            "not-top-3 bot should not enter Defending"
        );

        let inputs_top3 = BehaviorInputs {
            is_top_3_defender: true,
            prefers_rush: false,
            ..inputs_not_top3
        };
        // Need DEFEND_ENTER_TICKS ticks.
        for _ in 0..DEFEND_ENTER_TICKS {
            let _ = tick(&mut state, &inputs_top3);
        }
        assert_eq!(state.state, State::Defending);
    }

    #[test]
    fn committing_flag_flips_at_700m_boundary() {
        let mut state = BehaviorState {
            state: State::Transiting { committing: false },
            path_target: Vec2::new(0.0, -1000.0), // enemy base
            ..Default::default()
        };
        let terrain = common::terrain::Terrain::new();
        let inputs_far = BehaviorInputs {
            ship_pos: Vec2::new(0.0, 500.0), // ~1500 m from enemy base
            ship_heading: Vec2::new(0.0, -1.0),
            ship_speed: 10.0,
            ship_length: 30.0,
            closest_enemy: None,
            nearest_teammate: None,
            incoming_torpedo: None,
            own_base: Vec2::new(0.0, 1000.0),
            enemy_base: Vec2::new(0.0, -1000.0),
            own_base_capture_ms: 0,
            is_top_3_defender: false,
            prefers_rush: false,
            terrain: &terrain,
            world_radius: 3000.0,
        };
        let _ = tick(&mut state, &inputs_far);
        assert_eq!(state.state, State::Transiting { committing: false });

        let inputs_close = BehaviorInputs {
            ship_pos: Vec2::new(0.0, -500.0), // 500 m from enemy base
            ..inputs_far
        };
        let _ = tick(&mut state, &inputs_close);
        assert_eq!(state.state, State::Transiting { committing: true });
    }
}

/// Plan a new path from `from` to `to` using the given flow field,
/// writing it into `state.path`. Called by the outer update when a
/// replan is needed (state change, target change, or path drift).
pub fn plan_path(
    state: &mut BehaviorState,
    flow: &FlowField,
    terrain: &Terrain,
    from: Vec2,
    to: Vec2,
) {
    use crate::bot_pathfinder::trace_path;
    state.path = trace_path(flow, terrain, from, to, 80.0, 25);
    state.path_target = to;
}
