// SPDX-FileCopyrightText: 2026 scotthurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Custom ship picker for Capture the Area mode.
//!
//! A clean two-panel layout: a ship grid on the left filtered by the
//! currently-selected level, and a stat detail panel on the right that
//! updates when the player taps a ship. Level arrows flip between
//! levels 1..MAX so all ships are reachable. Nothing mentions
//! "respawning" — this is an explicit loadout pick before the match
//! starts.

use crate::ui::sprite::Sprite;
use common::entity::{EntityData, EntityKind, EntitySubKind, EntityType};
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct ShipPickerProps {
    /// Currently-selected ship, if any. Controls the highlighted card
    /// and the detail panel content.
    pub selected: Option<EntityType>,
    /// Fires when the player taps a ship card.
    pub on_pick: Callback<EntityType>,
    /// Fires when the player taps "Back".
    pub on_back: Callback<MouseEvent>,
    /// Fires when the player taps "Start Game". The parent is
    /// responsible for actually dispatching the Spawn command.
    pub on_start: Callback<MouseEvent>,
}

#[function_component(ShipPicker)]
pub fn ship_picker(props: &ShipPickerProps) -> Html {
    // Default to the level of the currently-selected ship, falling
    // back to level 1 if nothing is picked yet.
    let initial_level = props.selected.map(|e| e.data().level).unwrap_or(1);
    let level = use_state(|| initial_level);

    let max_level = EntityData::MAX_BOAT_LEVEL;

    // All spawnable, non-npc ships at the current level. Sorted by
    // name so the grid layout is stable across re-renders.
    let mut ships: Vec<EntityType> = EntityType::spawn_options(u32::MAX, false)
        .filter(|e| e.data().level == *level)
        .collect();
    ships.sort_by_key(|e| e.data().label);

    // Level nav wraps around: going back from level 1 lands on max,
    // and forward from max lands on 1. No one-way doors.
    let on_prev = {
        let level = level.clone();
        Callback::from(move |_: MouseEvent| {
            let new = if *level > 1 { *level - 1 } else { max_level };
            level.set(new);
        })
    };
    let on_next = {
        let level = level.clone();
        Callback::from(move |_: MouseEvent| {
            let new = if *level < max_level { *level + 1 } else { 1 };
            level.set(new);
        })
    };

    let has_selection = props.selected.is_some();

    // Container — wargame panel frame. Width responsive so the picker
    // fits narrow browser windows without clipping the detail panel.
    // `width: min(1040px, calc(100vw - 40px))` targets 1040px on wide
    // screens and shrinks to 20px-margin of the viewport on narrow
    // ones. overflow-x: auto is a safety net if inner content still
    // exceeds the available space (e.g. on very small phones).
    html! {
        <div style="display: flex; flex-direction: column; align-items: stretch; gap: 24px; padding: 36px 48px; background: rgba(15,23,42,0.92); border: 1px solid rgba(148,163,184,0.4); border-left: 4px solid #4ADE80; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; box-shadow: 0 8px 32px rgba(0,0,0,0.6); width: min(1040px, calc(100vw - 40px)); max-height: 90vh; overflow-y: auto; overflow-x: auto; box-sizing: border-box;">

            // Header row: back button + title
            <div style="display: flex; align-items: center; justify-content: space-between;">
                <button
                    style="display: flex; align-items: center; min-width: 120px; height: 44px; padding: 0 20px; background: rgba(15,23,42,0.92); color: #94A3B8; border: 1px solid rgba(148,163,184,0.3); border-left: 3px solid #64748B; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 13px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase; cursor: pointer;"
                    onclick={props.on_back.clone()}
                >
                    {"< Back"}
                </button>
                <div style="color: #FCD34D; font-size: 26px; font-weight: 700; letter-spacing: 5px; text-transform: uppercase; text-shadow: 0 2px 6px rgba(0,0,0,0.6);">
                    {"Select Your Ship"}
                </div>
                <div style="min-width: 120px;"></div> // spacer to balance the Back button
            </div>

            // Level nav: < Level N >   (wraps around at both ends)
            <div style="display: flex; align-items: center; justify-content: center; gap: 20px; padding: 10px 0; border-top: 1px solid rgba(148,163,184,0.15); border-bottom: 1px solid rgba(148,163,184,0.15);">
                <button
                    style="width: 48px; height: 48px; background: rgba(15,23,42,0.92); color: #4ADE80; border: 1px solid rgba(148,163,184,0.3); border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 22px; font-weight: 700; cursor: pointer;"
                    onclick={on_prev}
                >
                    {"<"}
                </button>
                <div style="color: #E2E8F0; font-size: 18px; font-weight: 700; letter-spacing: 3px; text-transform: uppercase; min-width: 180px; text-align: center;">
                    {format!("Level {} / {}", *level, max_level)}
                </div>
                <button
                    style="width: 48px; height: 48px; background: rgba(15,23,42,0.92); color: #4ADE80; border: 1px solid rgba(148,163,184,0.3); border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 22px; font-weight: 700; cursor: pointer;"
                    onclick={on_next}
                >
                    {">"}
                </button>
            </div>

            // Ship grid — larger cards, more visible per row
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 20px; min-height: 260px;">
                {
                    if ships.is_empty() {
                        html! {
                            <div style="grid-column: 1 / -1; color: #64748B; font-size: 13px; text-align: center; padding: 64px 0;">
                                {"(no ships at this level)"}
                            </div>
                        }
                    } else {
                        ships.iter().copied().map(|ship| {
                            let is_selected = props.selected == Some(ship);
                            let on_click = {
                                let cb = props.on_pick.clone();
                                Callback::from(move |_: MouseEvent| cb.emit(ship))
                            };
                            render_ship_card(ship, is_selected, on_click)
                        }).collect::<Html>()
                    }
                }
            </div>

            // Detail panel
            { render_detail_panel(props.selected) }

            // Start Game button
            <div style="display: flex; justify-content: center;">
                <button
                    style={format!(
                        "display: flex; align-items: center; justify-content: center; min-width: 260px; height: 60px; padding: 0 48px; background: rgba(15,23,42,0.92); color: {}; border: 1px solid {}; border-left: 3px solid {}; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 20px; font-weight: 700; letter-spacing: 4px; text-transform: uppercase; cursor: {}; box-shadow: 0 2px 8px rgba(0,0,0,0.5);",
                        if has_selection { "#4ADE80" } else { "#475569" },
                        if has_selection { "rgba(34,197,94,0.4)" } else { "rgba(71,85,105,0.3)" },
                        if has_selection { "#22C55E" } else { "#475569" },
                        if has_selection { "pointer" } else { "not-allowed" })}
                    onclick={props.on_start.clone()}
                    disabled={!has_selection}
                >
                    {"Start Game"}
                </button>
            </div>
        </div>
    }
}

fn render_ship_card(ship: EntityType, selected: bool, on_click: Callback<MouseEvent>) -> Html {
    let card_style = if selected {
        "display: flex; flex-direction: column; align-items: center; justify-content: space-between; gap: 10px; padding: 18px 12px; min-height: 180px; background: rgba(74,222,128,0.12); border: 2px solid #22C55E; border-radius: 2px; cursor: pointer; transition: all 0.15s ease-out;"
    } else {
        "display: flex; flex-direction: column; align-items: center; justify-content: space-between; gap: 10px; padding: 18px 12px; min-height: 180px; background: rgba(15,23,42,0.6); border: 1px solid rgba(148,163,184,0.2); border-radius: 2px; cursor: pointer; transition: all 0.15s ease-out;"
    };
    let label_color = if selected { "#4ADE80" } else { "#E2E8F0" };
    let class_color = if selected { "#4ADE80" } else { "#94A3B8" };
    let data = ship.data();

    html! {
        <div style={card_style} onclick={on_click}>
            // Larger sprite zoom — was 0.55, now 0.75 for a more legible
            // silhouette in the picker.
            <div style="display: flex; align-items: center; justify-content: center; flex: 1; zoom: 0.75;">
                <Sprite entity_type={ship}/>
            </div>
            <div style="display: flex; flex-direction: column; align-items: center; gap: 4px;">
                <div style={format!("color: {}; font-size: 13px; font-weight: 700; letter-spacing: 1.5px; text-transform: uppercase; text-align: center;", label_color)}>
                    {data.label}
                </div>
                <div style={format!("color: {}; font-size: 10px; font-weight: 400; letter-spacing: 0.5px; text-transform: uppercase;", class_color)}>
                    {subkind_label(data.sub_kind)}
                </div>
            </div>
        </div>
    }
}

fn render_detail_panel(selected: Option<EntityType>) -> Html {
    match selected {
        None => html! {
            <div style="display: flex; align-items: center; justify-content: center; min-height: 160px; padding: 24px; background: rgba(15,23,42,0.5); border: 1px dashed rgba(148,163,184,0.3); border-radius: 2px; color: #64748B; font-size: 14px; letter-spacing: 3px; text-transform: uppercase;">
                {"Tap a ship to see its stats"}
            </div>
        },
        Some(ship) => {
            let data = ship.data();
            let stats = ShipStats::from_data(data);

            html! {
                // minmax(0, ...) lets both columns shrink below their
                // intrinsic content width instead of forcing the
                // container wider than its parent. Without this the
                // sprite + "SELECT YOUR SHIP" header + stat grid add up
                // to >1040px on some levels and force a horizontal
                // scroll that hides the detail panel's left edge.
                <div style="display: grid; grid-template-columns: minmax(0, 2fr) minmax(0, 5fr); gap: 24px; padding: 24px 28px; background: rgba(15,23,42,0.55); border: 1px solid rgba(74,222,128,0.3); border-left: 3px solid #4ADE80; border-radius: 2px; min-width: 0;">
                    // Left column: name + class + big sprite
                    <div style="display: flex; flex-direction: column; gap: 12px; min-width: 0; overflow: hidden;">
                        <div style="min-width: 0;">
                            <div style="color: #FCD34D; font-size: 24px; font-weight: 700; letter-spacing: 4px; text-transform: uppercase; text-shadow: 0 2px 4px rgba(0,0,0,0.6); word-break: break-word;">
                                {data.label}
                            </div>
                            <div style="margin-top: 4px; color: #94A3B8; font-size: 12px; font-weight: 400; letter-spacing: 1.5px; text-transform: uppercase;">
                                {format!("Level {} {}", data.level, subkind_label(data.sub_kind))}
                            </div>
                        </div>
                        <div style="display: flex; align-items: center; justify-content: center; padding: 8px; background: rgba(15,23,42,0.35); border-radius: 2px; min-height: 100px; zoom: 0.75; overflow: hidden;">
                            <Sprite entity_type={ship}/>
                        </div>
                    </div>
                    // Right column: stat grid. minmax(0, 1fr) on each
                    // cell keeps the stat columns from stretching the
                    // parent grid when labels are long.
                    <div style="display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px 20px; align-content: start; min-width: 0;">
                        { render_stat("Speed",      &format!("{:.0} kn", stats.speed_knots)) }
                        { render_stat("Health",     &format!("{:.0}", stats.health)) }
                        { render_stat("Length",     &format!("{:.0} m", stats.length_m)) }

                        { render_stat("Turrets",    &format!("{}", stats.turret_count)) }
                        { render_stat("Weapons",    &format!("{}", stats.armament_count)) }
                        { render_stat("Range",      &format!("{:.0} m", stats.range_m)) }

                        { render_stat("Guns",       &format!("{}", stats.guns)) }
                        { render_stat("Torpedoes",  &format!("{}", stats.torpedoes)) }
                        { render_stat("Missiles",   &format!("{}", stats.missiles)) }

                        { render_stat("Aircraft",   &format!("{}", stats.aircraft)) }
                        { render_stat("Mines",      &format!("{}", stats.mines)) }
                        { render_stat("Vision",     &format!("{:.0} m", stats.vision_m)) }
                    </div>
                </div>
            }
        }
    }
}

/// Rolled-up stat snapshot for the detail panel. Keeps the render
/// function readable and gives us a single place to tweak how each
/// number is derived from EntityData.
struct ShipStats {
    speed_knots: f32,
    length_m: f32,
    health: f32,
    turret_count: usize,
    armament_count: usize,
    range_m: f32,
    vision_m: f32,
    guns: usize,
    torpedoes: usize,
    missiles: usize,
    aircraft: usize,
    mines: usize,
}

impl ShipStats {
    fn from_data(data: &'static EntityData) -> Self {
        use EntitySubKind::*;

        let mut guns = 0usize;
        let mut torpedoes = 0usize;
        let mut missiles = 0usize;
        let mut aircraft = 0usize;
        let mut mines = 0usize;

        for armament in data.armaments {
            match armament.entity_type.data().sub_kind {
                Shell | Gun => guns += 1,
                Torpedo => torpedoes += 1,
                Missile | Rocket | RocketTorpedo | Sam => missiles += 1,
                Plane | Heli => aircraft += 1,
                Mine | DepthCharge => mines += 1,
                _ => {}
            }
        }

        Self {
            speed_knots: data.speed.to_knots(),
            length_m: data.length,
            health: data.damage,
            turret_count: data.turrets.len(),
            armament_count: data.armaments.len(),
            range_m: data.range,
            vision_m: data.sensors.visual.range,
            guns,
            torpedoes,
            missiles,
            aircraft,
            mines,
        }
    }
}

fn render_stat(label: &str, value: &str) -> Html {
    html! {
        <div style="display: flex; justify-content: space-between; gap: 12px; font-size: 12px; letter-spacing: 1px; padding: 4px 0; border-bottom: 1px solid rgba(148,163,184,0.08);">
            <span style="color: #64748B; text-transform: uppercase;">{label}</span>
            <span style="color: #E2E8F0; font-weight: 700; text-align: right;">{value}</span>
        </div>
    }
}

fn subkind_label(sub: EntitySubKind) -> &'static str {
    use EntitySubKind::*;
    match sub {
        Battleship => "Battleship",
        Carrier => "Carrier",
        Corvette => "Corvette",
        Cruiser => "Cruiser",
        Destroyer => "Destroyer",
        Dreadnought => "Dreadnought",
        Hovercraft => "Hovercraft",
        Icebreaker => "Icebreaker",
        Lcs => "LCS",
        MissileBoat => "Missile Boat",
        Minelayer => "Minelayer",
        Mtb => "MTB",
        Pirate => "Pirate",
        Ram => "Ram",
        Submarine => "Submarine",
        Tanker => "Tanker",
        _ => "Warship",
    }
}

// Convenience: keep EntityKind in scope for the compiler even though
// we only match on sub_kind here. Helps future expansion if we start
// filtering by kind.
#[allow(dead_code)]
fn _kind_noop(_k: EntityKind) {}
