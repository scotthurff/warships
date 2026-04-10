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

    let on_prev = {
        let level = level.clone();
        Callback::from(move |_: MouseEvent| {
            if *level > 1 {
                level.set(*level - 1);
            }
        })
    };
    let on_next = {
        let level = level.clone();
        Callback::from(move |_: MouseEvent| {
            if *level < max_level {
                level.set(*level + 1);
            }
        })
    };

    let has_selection = props.selected.is_some();

    // Container — wargame panel frame.
    html! {
        <div style="display: flex; flex-direction: column; align-items: stretch; gap: 20px; padding: 32px 40px; background: rgba(15,23,42,0.92); border: 1px solid rgba(148,163,184,0.4); border-left: 4px solid #4ADE80; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; box-shadow: 0 8px 32px rgba(0,0,0,0.6); min-width: 720px; max-width: 90vw;">

            // Header row: back button + title
            <div style="display: flex; align-items: center; justify-content: space-between;">
                <button
                    style="display: flex; align-items: center; min-width: 100px; height: 40px; padding: 0 18px; background: rgba(15,23,42,0.92); color: #94A3B8; border: 1px solid rgba(148,163,184,0.3); border-left: 3px solid #64748B; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 12px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase; cursor: pointer;"
                    onclick={props.on_back.clone()}
                >
                    {"< Back"}
                </button>
                <div style="color: #FCD34D; font-size: 20px; font-weight: 700; letter-spacing: 4px; text-transform: uppercase; text-shadow: 0 2px 6px rgba(0,0,0,0.6);">
                    {"Select Your Ship"}
                </div>
                <div style="min-width: 100px;"></div> // spacer to balance the Back button
            </div>

            // Level nav: < Level N >
            <div style="display: flex; align-items: center; justify-content: center; gap: 16px; padding: 8px 0; border-top: 1px solid rgba(148,163,184,0.15); border-bottom: 1px solid rgba(148,163,184,0.15);">
                <button
                    style={format!("width: 40px; height: 40px; background: rgba(15,23,42,0.92); color: {}; border: 1px solid rgba(148,163,184,0.3); border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 18px; font-weight: 700; cursor: {};",
                        if *level == 1 { "#475569" } else { "#4ADE80" },
                        if *level == 1 { "not-allowed" } else { "pointer" })}
                    onclick={on_prev}
                    disabled={*level == 1}
                >
                    {"<"}
                </button>
                <div style="color: #E2E8F0; font-size: 16px; font-weight: 700; letter-spacing: 3px; text-transform: uppercase; min-width: 140px; text-align: center;">
                    {format!("Level {} / {}", *level, max_level)}
                </div>
                <button
                    style={format!("width: 40px; height: 40px; background: rgba(15,23,42,0.92); color: {}; border: 1px solid rgba(148,163,184,0.3); border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 18px; font-weight: 700; cursor: {};",
                        if *level == max_level { "#475569" } else { "#4ADE80" },
                        if *level == max_level { "not-allowed" } else { "pointer" })}
                    onclick={on_next}
                    disabled={*level == max_level}
                >
                    {">"}
                </button>
            </div>

            // Ship grid
            <div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 16px; min-height: 200px;">
                {
                    if ships.is_empty() {
                        html! {
                            <div style="grid-column: 1 / -1; color: #64748B; font-size: 12px; text-align: center; padding: 48px 0;">
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
                        "display: flex; align-items: center; justify-content: center; min-width: 220px; height: 56px; padding: 0 40px; background: rgba(15,23,42,0.92); color: {}; border: 1px solid {}; border-left: 3px solid {}; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 18px; font-weight: 700; letter-spacing: 3px; text-transform: uppercase; cursor: {}; box-shadow: 0 2px 8px rgba(0,0,0,0.5);",
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
        "display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 12px 8px; background: rgba(74,222,128,0.12); border: 2px solid #22C55E; border-radius: 2px; cursor: pointer; transition: all 0.15s ease-out;"
    } else {
        "display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 12px 8px; background: rgba(15,23,42,0.6); border: 1px solid rgba(148,163,184,0.2); border-radius: 2px; cursor: pointer; transition: all 0.15s ease-out;"
    };
    let label_color = if selected { "#4ADE80" } else { "#E2E8F0" };
    let class_color = if selected { "#4ADE80" } else { "#94A3B8" };
    let data = ship.data();

    html! {
        <div style={card_style} onclick={on_click}>
            <div style="zoom: 0.55;">
                <Sprite entity_type={ship}/>
            </div>
            <div style={format!("color: {}; font-size: 11px; font-weight: 700; letter-spacing: 1px; text-transform: uppercase; text-align: center;", label_color)}>
                {data.label}
            </div>
            <div style={format!("color: {}; font-size: 9px; font-weight: 400; letter-spacing: 0.5px; text-transform: uppercase;", class_color)}>
                {subkind_label(data.sub_kind)}
            </div>
        </div>
    }
}

fn render_detail_panel(selected: Option<EntityType>) -> Html {
    match selected {
        None => html! {
            <div style="display: flex; align-items: center; justify-content: center; min-height: 80px; padding: 18px; background: rgba(15,23,42,0.5); border: 1px dashed rgba(148,163,184,0.3); border-radius: 2px; color: #64748B; font-size: 12px; letter-spacing: 2px; text-transform: uppercase;">
                {"Tap a ship to see its stats"}
            </div>
        },
        Some(ship) => {
            let data = ship.data();
            let speed_knots = data.speed.to_knots();
            let armament_count = data.armaments.len();
            html! {
                <div style="display: grid; grid-template-columns: 2fr 3fr; gap: 20px; padding: 18px 20px; background: rgba(15,23,42,0.5); border: 1px solid rgba(74,222,128,0.3); border-left: 3px solid #4ADE80; border-radius: 2px;">
                    // Left column: name + class
                    <div style="display: flex; flex-direction: column; gap: 4px;">
                        <div style="color: #FCD34D; font-size: 18px; font-weight: 700; letter-spacing: 3px; text-transform: uppercase;">
                            {data.label}
                        </div>
                        <div style="color: #94A3B8; font-size: 11px; font-weight: 400; letter-spacing: 1px; text-transform: uppercase;">
                            {format!("Level {} {}", data.level, subkind_label(data.sub_kind))}
                        </div>
                    </div>
                    // Right column: stats
                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px 20px; align-items: start;">
                        { render_stat("Speed", &format!("{:.0} kn", speed_knots)) }
                        { render_stat("Length", &format!("{:.0} m", data.length)) }
                        { render_stat("Armaments", &format!("{}", armament_count)) }
                        { render_stat("Class", subkind_label(data.sub_kind)) }
                    </div>
                </div>
            }
        }
    }
}

fn render_stat(label: &str, value: &str) -> Html {
    html! {
        <div style="display: flex; justify-content: space-between; gap: 12px; font-size: 11px; letter-spacing: 1px;">
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
