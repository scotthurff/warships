// SPDX-FileCopyrightText: 2026 scotthurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Capture the Area minimap. Bottom-right of the screen during CTA
//! matches. Shows the full 1200-radius arena with:
//!
//!  - a translucent circle for the arena boundary
//!  - Blue / Red base discs at (0, ±500)
//!  - a dot for every visible ship, colored by team
//!  - a yellow ring marking the local player's ship
//!
//! Hidden in Free Roam.

use common::protocol::MatchTeam;
use kodiak_client::glam::Vec2;
use yew::prelude::*;

/// One ship on the minimap. Built server-side? No — built client-side
/// from `state.game.contacts` cross-referenced against `match_update`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinimapEntry {
    pub pos: Vec2,
    pub team: Option<MatchTeam>,
    pub is_you: bool,
}

#[derive(Properties, PartialEq)]
pub struct MinimapProps {
    pub entries: Vec<MinimapEntry>,
}

/// Minimap panel size in pixels (square).
const MINIMAP_PX: f32 = 200.0;
/// Arena radius in world units. Mirrors `server::match_state::ArenaLayout::DEFAULT.arena_radius`.
const ARENA_RADIUS: f32 = 1500.0;
/// Base radius in world units.
const BASE_RADIUS: f32 = 250.0;

#[function_component(Minimap)]
pub fn minimap(props: &MinimapProps) -> Html {
    // Convert a world (x, y) to a percent-of-minimap. (0,0) maps to
    // (50%, 50%). Positive Y is up in world coords but CSS's top
    // origin flips it.
    let to_percent = |world: Vec2| -> (f32, f32) {
        let half = ARENA_RADIUS;
        let x_pct = 50.0 + (world.x / (2.0 * half)) * 100.0;
        let y_pct = 50.0 - (world.y / (2.0 * half)) * 100.0;
        (x_pct.clamp(0.0, 100.0), y_pct.clamp(0.0, 100.0))
    };

    let (blue_x, blue_y) = to_percent(Vec2::new(0.0, 500.0));
    let (red_x, red_y) = to_percent(Vec2::new(0.0, -500.0));
    // Arena circle is approximately the inscribed circle of the square
    // panel, scaled so (0,0) → center and ARENA_RADIUS → outer ring.
    let arena_pct: f32 = 100.0; // fill the box — arena boundary = edge of minimap

    let container_style = format!(
        "position: fixed; bottom: 16px; right: 16px; width: {px}px; height: {px}px; background: rgba(15,23,42,0.92); border: 1px solid rgba(148,163,184,0.4); border-left: 3px solid #4ADE80; border-radius: 2px; box-shadow: 0 4px 16px rgba(0,0,0,0.6); overflow: hidden; z-index: 100;",
        px = MINIMAP_PX
    );

    let arena_circle_style = format!(
        "position: absolute; top: 0; left: 0; width: {pct}%; height: {pct}%; border: 1px dashed rgba(148,163,184,0.35); border-radius: 50%; box-sizing: border-box; pointer-events: none;",
        pct = arena_pct
    );

    // Pre-compute base disc sizes + style strings so we don't need
    // bare `let` blocks inside the html! macro (which Yew doesn't
    // allow).
    let base_disc_px = (BASE_RADIUS / (2.0 * ARENA_RADIUS)) * MINIMAP_PX * 2.0;
    let blue_base_style = format!(
        "position: absolute; left: {x}%; top: {y}%; width: {d}px; height: {d}px; transform: translate(-50%, -50%); border-radius: 50%; background: rgba(96,165,250,0.25); border: 1px solid rgba(96,165,250,0.8); pointer-events: none;",
        x = blue_x, y = blue_y, d = base_disc_px
    );
    let red_base_style = format!(
        "position: absolute; left: {x}%; top: {y}%; width: {d}px; height: {d}px; transform: translate(-50%, -50%); border-radius: 50%; background: rgba(248,113,113,0.25); border: 1px solid rgba(248,113,113,0.8); pointer-events: none;",
        x = red_x, y = red_y, d = base_disc_px
    );

    html! {
        <div style={container_style}>
            // Dashed arena boundary ring
            <div style={arena_circle_style}></div>

            // Blue base disc
            <div style={blue_base_style}></div>

            // Red base disc
            <div style={red_base_style}></div>

            // Ship dots
            { for props.entries.iter().map(|entry| render_ship_dot(entry, &to_percent)) }

            // Label
            <div style="position: absolute; bottom: 4px; left: 6px; color: #94A3B8; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 9px; font-weight: 700; letter-spacing: 1.5px; text-transform: uppercase; pointer-events: none;">
                {"Arena"}
            </div>
        </div>
    }
}

fn render_ship_dot(entry: &MinimapEntry, to_percent: &impl Fn(Vec2) -> (f32, f32)) -> Html {
    let (x, y) = to_percent(entry.pos);
    let color = match entry.team {
        Some(MatchTeam::Blue) => "#60A5FA",
        Some(MatchTeam::Red) => "#F87171",
        None => "#94A3B8",
    };
    let size = if entry.is_you { 10.0 } else { 6.0 };
    let (outline_w, outline_color) = if entry.is_you {
        (2.0, "#FCD34D")
    } else {
        (0.0, "transparent")
    };
    let style = format!(
        "position: absolute; left: {x}%; top: {y}%; width: {s}px; height: {s}px; transform: translate(-50%, -50%); border-radius: 50%; background: {bg}; border: {bw}px solid {oc}; box-shadow: 0 0 4px {bg}; pointer-events: none;",
        x = x, y = y, s = size, bg = color, bw = outline_w, oc = outline_color
    );
    html! { <div style={style}></div> }
}
