// SPDX-FileCopyrightText: 2026 Scott Hurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! CTA-arena terrain shaping.
//!
//! The procedural noise generator (`crate::noise`) produces pleasant
//! islands everywhere, including directly in the navigation corridor
//! between CTA bases. At the 2× expanded arena scale (bases at
//! y = ±1000), ships still find the corridor too densely packed —
//! non-holonomic turn radii of 100–270 m can't thread the gaps at
//! cruise speed.
//!
//! This module sparsens the corridor at CTA match start: walk a 20 m
//! grid inside a stadium shape spanning the two bases, and for ~70%
//! of the LAND cells write them down to shallow water. Keeps ~30% of
//! blocking islands as visible terrain so the arena doesn't look like
//! the reverted "flatten everything" carve (`ae1026a`, reverted in
//! `b32bed0`).
//!
//! See `plans/cta-arena-expand-and-sparsen.md` for the full design.

use crate::match_state::ArenaLayout;
use common::altitude::Altitude;
use common::terrain::{Terrain, TerrainMutation, SAND_LEVEL};
use kodiak_server::glam::Vec2;

/// Sample granularity of the sparsen walk. 20 m is finer than the
/// underlying terrain cell size (25 m), so every terrain cell inside
/// the stadium gets at least one mutation applied. Going finer would
/// just re-hit cells that are already sparsened.
const WALK_STEP: f32 = 20.0;

/// Half-width of the stadium corridor, perpendicular to the base-to-
/// base axis. 700 m = 4× the widest ship length (Iowa at 270 m) and
/// wide enough that the flow field always has at least one clear
/// path from base to base even with 30% of islands surviving.
const STADIUM_HALF_WIDTH: f32 = 700.0;

/// Sparsening survival rate at the chosen hash cutoff. 0xB3 / 0xFF
/// ≈ 0.30 — i.e. ~70% of land cells inside the stadium are flattened
/// to water, ~30% kept as visible islands. Tune up (0xCC → ~0.20
/// survival) if terrain is still too thick; tune down (0xA0 → ~0.37)
/// if the arena looks sparse.
const KEEP_CUTOFF: u32 = 0xB3;

/// Flatten the CTA navigation corridor between `layout.blue_base` and
/// `layout.red_base`. Safe to call any time the terrain can be
/// mutated — intended to run once per match, immediately before
/// `FlowField::build` so the routing sees the sparsened state.
///
/// Synchronous direct-write path: each mutation is applied immediately
/// via `Terrain::modify` (which writes through `terrain.set`). Does
/// NOT touch the per-tick `TerrainMutation` queue in
/// `world_physics.rs` — that queue is for in-game ship-terrain
/// damage; match-start bulk edits bypass it.
pub fn sparsen_cta_corridor(terrain: &mut Terrain, layout: &ArenaLayout) {
    let axis_start = layout.blue_base;
    let axis_end = layout.red_base;
    // Stadium = rectangle + two half-disks. Bounding box:
    //   x ∈ [-STADIUM_HALF_WIDTH, +STADIUM_HALF_WIDTH]
    //   y ∈ [min(blue.y, red.y) - STADIUM_HALF_WIDTH, max + STADIUM_HALF_WIDTH]
    // (The half-disk caps extend STADIUM_HALF_WIDTH past each base.)
    let y_lo = axis_start.y.min(axis_end.y) - STADIUM_HALF_WIDTH;
    let y_hi = axis_start.y.max(axis_end.y) + STADIUM_HALF_WIDTH;
    let x_lo = -STADIUM_HALF_WIDTH;
    let x_hi = STADIUM_HALF_WIDTH;

    let mut x = x_lo;
    while x <= x_hi {
        let mut y = y_lo;
        while y <= y_hi {
            let pos = Vec2::new(x, y);
            if inside_stadium(pos, axis_start, axis_end) {
                if !stadium_keep(x as i32, y as i32) {
                    // conditional_clamped: only mutate cells currently
                    // at or above SAND_LEVEL (land), and clamp the
                    // result into [MIN, -8] (shallow water). Water
                    // cells are untouched. Max-magnitude amount
                    // (-255) guarantees the clamp is hit in one pass.
                    let _ = terrain.modify(TerrainMutation::conditional_clamped(
                        pos,
                        -255.0,
                        SAND_LEVEL..=Altitude::MAX,
                        Altitude::MIN..=Altitude(-8),
                    ));
                }
            }
            y += WALK_STEP;
        }
        x += WALK_STEP;
    }
}

/// Stadium predicate: rectangle band plus two half-disk caps.
///
/// The axis is a line segment from `axis_start` to `axis_end`. A
/// point is inside the stadium iff its distance to the segment is
/// ≤ `STADIUM_HALF_WIDTH`.
fn inside_stadium(p: Vec2, axis_start: Vec2, axis_end: Vec2) -> bool {
    // Closest-point-on-segment distance. Standard formulation:
    // project p onto the axis, clamp the parameter to [0, 1].
    let ab = axis_end - axis_start;
    let ap = p - axis_start;
    let denom = ab.length_squared();
    let t = if denom > 0.0 {
        (ap.dot(ab) / denom).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let closest = axis_start + ab * t;
    (p - closest).length_squared() <= STADIUM_HALF_WIDTH * STADIUM_HALF_WIDTH
}

/// Deterministic ~30% survival hash for stadium cells.
///
/// MurmurHash3 fmix32 after combining both axes through different
/// multiplicative constants. Important property: neither axis can
/// zero-out the result. A naive `(x * K1) ^ (y * K2)` has a blind
/// spot at x=0 or y=0 (the corridor center axis is literally x=0,
/// which would produce visible streaks of all-water or all-land
/// right along the main nav spine). The rotate-left + fmix below
/// mixes both axes into every output bit.
fn stadium_keep(x: i32, y: i32) -> bool {
    let mut h = (x as u32).wrapping_mul(0xcc9e2d51)
        ^ ((y as u32).wrapping_mul(0x1b873593)).rotate_left(15);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^= h >> 16;
    (h & 0xFF) > KEEP_CUTOFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stadium_contains_endpoints_and_axis() {
        let blue = Vec2::new(0.0, 1000.0);
        let red = Vec2::new(0.0, -1000.0);
        assert!(inside_stadium(blue, blue, red));
        assert!(inside_stadium(red, blue, red));
        assert!(inside_stadium(Vec2::ZERO, blue, red));
        // Just inside the half-disk cap at blue base.
        assert!(inside_stadium(Vec2::new(0.0, 1500.0), blue, red));
        // Just inside the rectangle band.
        assert!(inside_stadium(Vec2::new(500.0, 0.0), blue, red));
    }

    #[test]
    fn stadium_excludes_beyond_width() {
        let blue = Vec2::new(0.0, 1000.0);
        let red = Vec2::new(0.0, -1000.0);
        // > STADIUM_HALF_WIDTH off-axis.
        assert!(!inside_stadium(Vec2::new(800.0, 0.0), blue, red));
        // Way past the half-disk cap.
        assert!(!inside_stadium(Vec2::new(0.0, 2500.0), blue, red));
    }

    #[test]
    fn stadium_keep_hash_has_no_axis_streaks() {
        // Guard against the naive (x*K1) ^ (y*K2) bug: when one
        // axis is zero, the output should still vary across the
        // other axis. If the hash were broken, every x along y=0
        // would return the same value.
        let mut survivals_y0 = 0;
        let mut survivals_x0 = 0;
        let n = 100;
        for i in 0..n {
            if stadium_keep(i, 0) {
                survivals_y0 += 1;
            }
            if stadium_keep(0, i) {
                survivals_x0 += 1;
            }
        }
        // At ~30% survival, over 100 samples we expect ~30
        // with wide variance. Fail if either axis is locked to
        // "all survive" (100) or "none survive" (0).
        assert!(
            (5..95).contains(&survivals_y0),
            "y=0 axis appears locked: {}/100 survivals",
            survivals_y0
        );
        assert!(
            (5..95).contains(&survivals_x0),
            "x=0 axis appears locked: {}/100 survivals",
            survivals_x0
        );
    }

    #[test]
    fn stadium_keep_is_deterministic() {
        assert_eq!(stadium_keep(42, 73), stadium_keep(42, 73));
        assert_eq!(stadium_keep(-100, 500), stadium_keep(-100, 500));
    }

    #[test]
    fn stadium_keep_survival_rate_is_approximately_thirty_percent() {
        // Sample a broad region of the coordinate plane and check
        // that the survival rate is in the right ballpark. If it
        // drifts far from 30%, the cutoff const is wrong.
        let mut survivals = 0;
        let mut total = 0;
        for x in (-500..=500).step_by(20) {
            for y in (-1000..=1000).step_by(20) {
                if stadium_keep(x, y) {
                    survivals += 1;
                }
                total += 1;
            }
        }
        let rate = survivals as f32 / total as f32;
        assert!(
            (0.22..0.38).contains(&rate),
            "survival rate {} outside expected ~30% range ({}/{})",
            rate,
            survivals,
            total
        );
    }
}
