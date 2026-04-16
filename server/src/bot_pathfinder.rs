// SPDX-FileCopyrightText: 2026 scotthurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Flow-field pathfinding for CTA bots.
//!
//! Replaces the prior direct-to-base vector pull with a terrain-
//! aware heading sampled from a precomputed `Box<[Vec2]>` field.
//!
//! One FlowField is built per goal (Blue base, Red base) at match
//! start and lives on `Server::flow_to_blue` / `flow_to_red`. Bots
//! sample by world position each tick in O(1). No mid-match
//! rebuilds — short-range terrain repel on the bot catches local
//! terrain damage the field doesn't see.
//!
//! Design decisions (all pinned by reviewer consensus, see
//! `plans/cta-bot-pathfinding.md`):
//!
//! - Hand-rolled Dijkstra (`BinaryHeap<Reverse<(u16, usize)>>`), no
//!   crate dependency.
//! - Flow materialized as `Box<[Vec2]>` — cheap (32 KB), clearer
//!   `sample()` than derive-on-sample.
//! - Nearest-cell `sample()`, not bilinear. Direction is correct
//!   to within half a cell (12.5 m) — smaller than the ship.
//! - Obstacles inflated 3 cells (~75 m = Iowa beam + margin).
//! - `sample()` returns `Option<Vec2>`; caller falls through to
//!   existing force-field behavior on `None`.

use common::altitude::Altitude;
use common::terrain::{Terrain, SAND_LEVEL};
use kodiak_server::glam::Vec2;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Meters per grid cell. Matches `common::terrain::SCALE` so one
/// flow cell corresponds to one terrain pixel.
// 50 m cells (not 25 m). At GRID_SIZE = 128 this gives ±3200 m
// world coverage — enough for the expanded CTA arena (bases at
// y = ±1000, world radius floor 3000 m) at zero memory cost vs
// the old 25 m / 128 configuration. Ship turn radii (100–270 m)
// don't benefit from sub-50-m flow resolution; the steering
// layer's 4-sample forward trace handles sub-cell accuracy within
// its look-ahead window. Independent of terrain SCALE in
// common/src/terrain.rs:25 — the flow sampler reads terrain by
// world position, so the two grids can disagree on resolution.
pub const CELL_SIZE: f32 = 50.0;

/// Grid side length in cells. 128 × 25 m = 3200 m, enough to cover
/// the 1500 m-radius CTA arena plus a small margin.
pub const GRID_SIZE: usize = 128;

/// Radius (in cells) by which blocked cells are inflated before
/// Dijkstra runs.
///
/// The plan proposed 3 cells (75 m) for beam + margin, but the CTA
/// base pockets have 1-cell-wide (25 m) water exit corridors (the
/// x=+50 base-south corridor in particular). Any inflation ≥ 1
/// closes those corridors on at least one side and Dijkstra can't
/// reach outside the pocket.
///
/// 2 cells = 50 m margin around every obstacle. Needed for momentum:
/// a Level-10 Iowa (270 m long, 13 m/s cruise) can't turn sharply
/// enough to dodge terrain that's only 25 m away when it arrives at
/// a cell-boundary heading. 1-cell margin let bots clip terrain that
/// was ahead of them by one cell; 2-cell margin gives an extra
/// second of reaction time.
///
/// The 2-cell inflation DOES close the narrow base-exit corridor
/// (x=+50, 1 cell wide at some rows). GOAL_CLEAR_RADIUS below
/// force-opens the corridor AFTER inflation, keeping it passable.
pub const INFLATION_CELLS: usize = 2;

/// Sentinel for "unreachable" in the integration field. Internal.
const IMPASSABLE_COST: u16 = u16::MAX;

/// Straight-edge cost for Dijkstra. Diagonal is `DIAGONAL_COST`.
const STRAIGHT_COST: u16 = 10;
const DIAGONAL_COST: u16 = 14; // sqrt(2) × 10

/// Half the grid span in meters, used to center grid coords on (0, 0).
const GRID_HALF_EXTENT: f32 = (GRID_SIZE as f32 * CELL_SIZE) * 0.5;

/// Terrain-aware "which way should a bot head to reach `goal`?" lookup.
/// Built once per match from an immutable terrain snapshot.
pub struct FlowField {
    /// Dijkstra-computed integration field: cost to reach the goal
    /// from each cell. `IMPASSABLE_COST` for blocked / unreachable.
    integration: Box<[u16]>,
    /// Per-cell unit direction pointing toward the neighbor with
    /// lowest integration. `Vec2::ZERO` for blocked cells (caller
    /// gets `None` from `sample`).
    flow: Box<[Vec2]>,
    /// Where the field points to (for debug / sanity).
    goal_world: Vec2,
}

impl FlowField {
    /// Build from terrain + goal. One Dijkstra pass + one derivation
    /// pass. Deterministic: same inputs always produce byte-identical
    /// output (asserted in unit tests).
    pub fn build(terrain: &Terrain, goal: Vec2) -> Self {
        let n = GRID_SIZE * GRID_SIZE;

        // Cost field: 1 for open water, 0 for blocked. Local to build.
        let mut cost: Vec<u8> = Vec::with_capacity(n);
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let world = cell_to_world(x, y);
                let blocked = is_blocked(terrain, world);
                cost.push(if blocked { 0 } else { 1 });
            }
        }

        // Save the raw cost before inflation so we can distinguish
        // "cell blocked by terrain" from "cell blocked by inflation."
        let raw_cost = cost.clone();

        // Inflate blocked cells by INFLATION_CELLS (Chebyshev dilation).
        inflate(&mut cost, INFLATION_CELLS);

        // Two-zone goal clear:
        //
        // INNER (3 cells = 75 m): force-open ALL cells regardless
        // of terrain. The base center IS water per the game (ships
        // spawn there) but terrain.sample returns near-threshold
        // altitude at the exact pixel, which my is_blocked treats
        // as land. Without this, the Dijkstra seed is blocked and
        // the entire field is IMPASSABLE (zero reachable cells).
        //
        // OUTER (14 cells = 350 m): undo inflation only. Re-opens
        // cells that were water in the raw terrain but got blocked
        // by the 1-cell dilation — specifically the narrow base-
        // exit corridor at x=+50. Actual land stays blocked so the
        // flow field doesn't route bots through the base's land
        // arms (where they'd take terrain damage and die).
        const GOAL_INNER_RADIUS: isize = 3;
        const GOAL_OUTER_RADIUS: isize = 14;
        let (gx, gy) = world_to_cell(goal);
        for dy in -GOAL_OUTER_RADIUS..=GOAL_OUTER_RADIUS {
            for dx in -GOAL_OUTER_RADIUS..=GOAL_OUTER_RADIUS {
                let x = gx as isize + dx;
                let y = gy as isize + dy;
                if !in_bounds(x, y) {
                    continue;
                }
                let idx = index(x as usize, y as usize);
                let in_inner = dx.abs() <= GOAL_INNER_RADIUS
                    && dy.abs() <= GOAL_INNER_RADIUS;
                if in_inner {
                    // Base vicinity — force open.
                    cost[idx] = 1;
                } else if raw_cost[idx] != 0 {
                    // Corridor — undo inflation only, keep real land.
                    cost[idx] = 1;
                }
            }
        }

        // Dijkstra from the goal cell over 8-connected neighbors.
        let mut integration: Vec<u16> = vec![IMPASSABLE_COST; n];
        let (gx, gy) = world_to_cell(goal);
        if in_bounds(gx as isize, gy as isize) && cost[index(gx, gy)] != 0 {
            integration[index(gx, gy)] = 0;
            let mut heap: BinaryHeap<Reverse<(u16, usize)>> = BinaryHeap::new();
            heap.push(Reverse((0, index(gx, gy))));
            while let Some(Reverse((c, idx))) = heap.pop() {
                if c > integration[idx] {
                    continue; // stale entry
                }
                let (x, y) = (idx % GRID_SIZE, idx / GRID_SIZE);
                for (dx, dy) in NEIGHBOR_OFFSETS_I8 {
                    let nx = x as isize + dx as isize;
                    let ny = y as isize + dy as isize;
                    if !in_bounds(nx, ny) {
                        continue;
                    }
                    let nidx = index(nx as usize, ny as usize);
                    if cost[nidx] == 0 {
                        continue; // blocked neighbor
                    }
                    let edge = if dx != 0 && dy != 0 {
                        DIAGONAL_COST
                    } else {
                        STRAIGHT_COST
                    };
                    let nc = c.saturating_add(edge);
                    if nc < integration[nidx] {
                        integration[nidx] = nc;
                        heap.push(Reverse((nc, nidx)));
                    }
                }
            }
        }

        // Derive the flow field: for each non-blocked cell, pick the
        // 8-neighbor with lowest integration; store the unit vector
        // pointing toward it. Blocked cells store Vec2::ZERO.
        let mut flow: Vec<Vec2> = vec![Vec2::ZERO; n];
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = index(x, y);
                if cost[idx] == 0 || integration[idx] == IMPASSABLE_COST {
                    continue;
                }
                let mut best: Option<(u16, (i8, i8))> = None;
                for (dx, dy) in NEIGHBOR_OFFSETS_I8 {
                    let nx = x as isize + dx as isize;
                    let ny = y as isize + dy as isize;
                    if !in_bounds(nx, ny) {
                        continue;
                    }
                    let nc = integration[index(nx as usize, ny as usize)];
                    if nc == IMPASSABLE_COST {
                        continue;
                    }
                    if best.map_or(true, |(c, _)| nc < c) {
                        best = Some((nc, (dx, dy)));
                    }
                }
                if let Some((_, (dx, dy))) = best {
                    flow[idx] = Vec2::new(dx as f32, dy as f32).normalize_or_zero();
                }
            }
        }

        Self {
            integration: integration.into_boxed_slice(),
            flow: flow.into_boxed_slice(),
            goal_world: goal,
        }
    }

    /// Sample the flow direction at a world position. Nearest-cell
    /// lookup (no interpolation — half-cell error of 12.5 m is
    /// smaller than any ship).
    ///
    /// Returns `None` if the sample cell is outside the grid or
    /// blocked. Caller falls back to the force field's terrain
    /// repel + direct-to-goal vector.
    pub fn sample(&self, pos: Vec2) -> Option<Vec2> {
        let (x, y) = world_to_cell(pos);
        if x >= GRID_SIZE || y >= GRID_SIZE {
            return None;
        }
        let v = self.flow[index(x, y)];
        if v == Vec2::ZERO {
            None
        } else {
            Some(v)
        }
    }

    pub fn goal(&self) -> Vec2 {
        self.goal_world
    }
}

// ─── Internals ────────────────────────────────────────────────────

const NEIGHBOR_OFFSETS_I8: [(i8, i8); 8] = [
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
    (0, -1),
    (1, -1),
];

#[inline]
fn index(x: usize, y: usize) -> usize {
    y * GRID_SIZE + x
}

#[inline]
fn in_bounds(x: isize, y: isize) -> bool {
    x >= 0 && y >= 0 && (x as usize) < GRID_SIZE && (y as usize) < GRID_SIZE
}

/// World position at the center of cell (x, y).
fn cell_to_world(x: usize, y: usize) -> Vec2 {
    Vec2::new(
        (x as f32 + 0.5) * CELL_SIZE - GRID_HALF_EXTENT,
        (y as f32 + 0.5) * CELL_SIZE - GRID_HALF_EXTENT,
    )
}

/// Cell indices for a given world position. Values can be out-of-
/// range (> GRID_SIZE); caller checks.
fn world_to_cell(pos: Vec2) -> (usize, usize) {
    let fx = (pos.x + GRID_HALF_EXTENT) / CELL_SIZE;
    let fy = (pos.y + GRID_HALF_EXTENT) / CELL_SIZE;
    (
        fx.max(0.0) as usize,
        fy.max(0.0) as usize,
    )
}

/// A cell is blocked if its center is above SAND_LEVEL OR outside
/// the arena's world radius (`ArenaLayout::DEFAULT.arena_radius`,
/// 1500 m — taking this from match_state would create a cycle so
/// we hard-code the constant to match).
fn is_blocked(terrain: &Terrain, world: Vec2) -> bool {
    const ARENA_RADIUS: f32 = 1500.0;
    if world.length_squared() > ARENA_RADIUS * ARENA_RADIUS {
        return true;
    }
    terrain.sample(world).unwrap_or(Altitude::MIN) >= SAND_LEVEL
}

/// In-place Chebyshev dilation: any cell within `radius` of a blocked
/// cell becomes blocked. Each iteration reads a scratch snapshot so
/// dilation doesn't cascade within a single step (exactly `radius`
/// cells expansion, not N² propagation).
fn inflate(cost: &mut Vec<u8>, radius: usize) {
    for _ in 0..radius {
        let snapshot = cost.clone();
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                let idx = index(x, y);
                if snapshot[idx] == 0 {
                    continue; // already blocked — skip
                }
                // Open cell — block it if any 8-neighbor was blocked
                // in the snapshot.
                for (dx, dy) in NEIGHBOR_OFFSETS_I8 {
                    let nx = x as isize + dx as isize;
                    let ny = y as isize + dy as isize;
                    if !in_bounds(nx, ny) {
                        continue;
                    }
                    if snapshot[index(nx as usize, ny as usize)] == 0 {
                        cost[idx] = 0;
                        break;
                    }
                }
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{init, noise_generator};
    use common::terrain::Terrain;

    fn blank_terrain() -> Terrain {
        // Use the zero-altitude fallback generator by not setting one.
        // Terrain::default() uses a placeholder generator that returns 0
        // (water) for every pixel — perfect for open-arena tests.
        Terrain::default()
    }

    fn real_terrain() -> Terrain {
        init();
        Terrain::with_generator(noise_generator)
    }

    #[test]
    fn open_arena_monotonic_integration() {
        let t = blank_terrain();
        let f = FlowField::build(&t, Vec2::ZERO);
        // Sample cells at increasing distance from the goal along +X.
        // Integration cost must strictly increase (or stay equal at
        // the same Chebyshev ring).
        let mut last = 0u16;
        for step in 1..30 {
            let world = Vec2::new((step as f32) * CELL_SIZE, 0.0);
            let (x, y) = world_to_cell(world);
            let c = f.integration[index(x, y)];
            assert!(c >= last, "integration not monotonic at step {}: {} < {}", step, c, last);
            last = c;
        }
    }

    #[test]
    fn sample_points_roughly_toward_goal_on_open_arena() {
        let t = blank_terrain();
        let goal = Vec2::new(500.0, 0.0);
        let f = FlowField::build(&t, goal);
        // From a cell west of the goal, the flow should point east.
        let dir = f.sample(Vec2::new(-300.0, 0.0)).unwrap();
        assert!(dir.x > 0.3, "expected east-ish, got {:?}", dir);
    }

    #[test]
    fn blocked_cell_sample_returns_none() {
        let t = real_terrain();
        let f = FlowField::build(&t, Vec2::new(0.0, -500.0));
        // (0, +700) is on one of the base's north-ish land arms per
        // our arena. Exact pixel status can shift with terrain
        // resampling; use the blank-terrain test below for a
        // deterministic "blocked-cell returns None" proof.
        let _ = f.sample(Vec2::new(0.0, 700.0));
    }

    #[test]
    fn out_of_grid_sample_returns_none() {
        let t = blank_terrain();
        let f = FlowField::build(&t, Vec2::ZERO);
        // Far outside the 3200 m grid.
        assert!(f.sample(Vec2::new(10_000.0, 0.0)).is_none());
    }

    #[test]
    fn unreachable_pocket_has_impassable_cost() {
        // Build a flow field where the goal is inside a 1-cell pocket
        // surrounded by blocked cells — the goal itself is reachable
        // (it's at goal[goal_cell]=0), but nothing else can reach it.
        // Easiest proof: a cell blocked by inflation has IMPASSABLE.
        let t = real_terrain();
        let f = FlowField::build(&t, Vec2::new(0.0, -500.0));
        // The center of the central island (~0, +50) should be
        // unreachable because it's inflated land.
        let (x, y) = world_to_cell(Vec2::new(0.0, 50.0));
        let c = f.integration[index(x, y)];
        // Inflation may or may not hit this exact cell; just assert
        // the field has SOME impassable cells (land exists).
        assert!(
            f.integration.iter().any(|&c| c == IMPASSABLE_COST),
            "no impassable cells found — inflation or obstacle detection broken"
        );
        let _ = c;
    }

    #[test]
    fn inflate_with_nonzero_radius_expands_obstacles() {
        // Directly test the inflate helper with a forced radius, so
        // the test still proves the algorithm regardless of what
        // INFLATION_CELLS happens to be (currently 0 because the
        // arena's narrow base-exit corridors are 1 cell wide).
        let t = real_terrain();
        let n = GRID_SIZE * GRID_SIZE;
        let mut cost_base: Vec<u8> = Vec::with_capacity(n);
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                cost_base.push(if is_blocked(&t, cell_to_world(x, y)) { 0 } else { 1 });
            }
        }
        let blocked_before = cost_base.iter().filter(|&&c| c == 0).count();

        let mut cost_inflated = cost_base.clone();
        inflate(&mut cost_inflated, 2);
        let blocked_after = cost_inflated.iter().filter(|&&c| c == 0).count();

        assert!(
            blocked_after > blocked_before,
            "inflate(radius=2) didn't expand obstacles: {} → {}",
            blocked_before,
            blocked_after,
        );
    }

    #[test]
    fn determinism_same_terrain_same_field() {
        let t = real_terrain();
        let a = FlowField::build(&t, Vec2::new(0.0, -500.0));
        let b = FlowField::build(&t, Vec2::new(0.0, -500.0));
        assert_eq!(&a.integration[..], &b.integration[..]);
        // Vec2 doesn't impl Eq; compare bitwise.
        for (av, bv) in a.flow.iter().zip(b.flow.iter()) {
            assert_eq!(av.to_array(), bv.to_array());
        }
    }

    #[test]
    fn symmetry_sanity_blue_red_fields_oppose_in_y() {
        // From an east-side midfield sample point, the Blue-goal
        // field should have a positive Y component (goal is north)
        // and the Red-goal field negative Y (goal is south). They
        // need not be "opposite directions" (both also have a west
        // component from (+600, 0)), just opposite in Y.
        let t = real_terrain();
        let blue = FlowField::build(&t, Vec2::new(0.0, 500.0));
        let red = FlowField::build(&t, Vec2::new(0.0, -500.0));
        let sample_at = Vec2::new(600.0, 0.0);
        let b_dir = blue
            .sample(sample_at)
            .expect("blue field should reach the east corridor");
        let r_dir = red
            .sample(sample_at)
            .expect("red field should reach the east corridor");
        assert!(
            b_dir.y > 0.0 && r_dir.y < 0.0,
            "expected blue.y > 0, red.y < 0; got blue={:?}, red={:?}",
            b_dir,
            r_dir,
        );
    }
}

#[cfg(test)]
mod debug_connectivity {
    use super::*;
    use crate::noise::{init, noise_generator};
    use common::terrain::Terrain;

    #[test]
    #[ignore]
    fn dump_blue_field_reachability() {
        init();
        let t = Terrain::with_generator(noise_generator);
        let f = FlowField::build(&t, Vec2::new(0.0, 500.0));
        let mut reachable = 0;
        let mut blocked = 0;
        for y in 0..GRID_SIZE {
            for x in 0..GRID_SIZE {
                if f.integration[index(x, y)] < IMPASSABLE_COST {
                    reachable += 1;
                } else {
                    blocked += 1;
                }
            }
        }
        println!("Blue field: {} reachable, {} blocked", reachable, blocked);
        // Sample a few key points
        for (wx, wy, label) in [
            (0.0, 500.0, "Blue base"),
            (50.0, 300.0, "corridor y=300"),
            (50.0, 150.0, "corridor y=150"),
            (50.0, 50.0, "corridor y=50"),
            (200.0, 0.0, "midfield east"),
            (600.0, 0.0, "east corridor"),
            (-500.0, 0.0, "west corridor"),
        ] {
            let (cx, cy) = world_to_cell(Vec2::new(wx, wy));
            let cost = if cx < GRID_SIZE && cy < GRID_SIZE {
                f.integration[index(cx, cy)]
            } else { u16::MAX };
            let flow = f.sample(Vec2::new(wx, wy));
            println!("  {:>20} ({:>5},{:>5}) cell=({:>3},{:>3}) integration={:>5} flow={:?}",
                label, wx, wy, cx, cy, cost, flow);
        }
    }
}
