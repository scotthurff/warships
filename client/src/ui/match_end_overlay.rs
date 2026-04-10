// SPDX-FileCopyrightText: 2026 scotthurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Full-screen results screen rendered when `MatchPhase::Ended { winner }`
//! is received. Shows the winning team banner, final scores, a sorted
//! per-player stats table, and Play Again / Quit to Title buttons.

use crate::ui::UiEvent;
use crate::Mk48Game;
use common::protocol::{MatchTeam, MatchUpdate, MatchWinner};
use kodiak_client::use_ui_event_callback;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct MatchEndOverlayProps {
    pub match_update: MatchUpdate,
    pub winner: MatchWinner,
}

#[function_component(MatchEndOverlay)]
pub fn match_end_overlay(props: &MatchEndOverlayProps) -> Html {
    let ui_event_callback = use_ui_event_callback::<Mk48Game>();

    let on_play_again = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| cb.emit(UiEvent::PlayAgain))
    };
    let on_quit = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| cb.emit(UiEvent::QuitToTitle))
    };

    let (winner_label, winner_color) = match props.winner {
        MatchWinner::Blue => ("BLUE TEAM WINS", "#60A5FA"),
        MatchWinner::Red => ("RED TEAM WINS", "#F87171"),
        MatchWinner::Draw => ("DRAW", "#FCD34D"),
    };

    let m = &props.match_update;

    html! {
        <div style="position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; background: rgba(15,23,42,0.85); backdrop-filter: blur(6px); z-index: 9999;">
            <div style="display: flex; flex-direction: column; align-items: stretch; gap: 24px; padding: 40px 56px; background: rgba(15,23,42,0.97); border: 1px solid rgba(148,163,184,0.4); border-left: 4px solid #FCD34D; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; box-shadow: 0 8px 32px rgba(0,0,0,0.7); min-width: 640px; max-width: 80vw;">
                // Winner banner
                <div style={format!("color: {}; font-family: 'Black Ops One', 'Menlo', monospace; font-size: 42px; font-weight: 900; letter-spacing: 6px; text-align: center; text-shadow: 0 2px 8px rgba(0,0,0,0.8);", winner_color)}>
                    {winner_label}
                </div>

                // Final scores row
                <div style="display: flex; align-items: center; justify-content: center; gap: 40px; font-size: 22px; font-weight: 700; letter-spacing: 3px; padding: 16px 0; border-top: 1px solid rgba(148,163,184,0.2); border-bottom: 1px solid rgba(148,163,184,0.2);">
                    <div style="color: #60A5FA;">{format!("BLUE {}", m.blue_score)}</div>
                    <div style="color: #94A3B8; font-size: 14px;">{"—"}</div>
                    <div style="color: #F87171;">{format!("{} RED", m.red_score)}</div>
                </div>

                // Sorted player stats table
                { render_stats_table(m) }

                // Buttons
                <div style="display: flex; gap: 16px; justify-content: center;">
                    <button
                        style="display: flex; align-items: center; justify-content: center; min-width: 180px; height: 52px; padding: 0 32px; background: rgba(15,23,42,0.92); color: #4ADE80; border: 1px solid rgba(34,197,94,0.4); border-left: 3px solid #22C55E; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 16px; font-weight: 700; letter-spacing: 3px; text-transform: uppercase; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);"
                        onclick={on_play_again}
                    >
                        {"Play Again"}
                    </button>
                    <button
                        style="display: flex; align-items: center; justify-content: center; min-width: 180px; height: 52px; padding: 0 32px; background: rgba(15,23,42,0.92); color: #94A3B8; border: 1px solid rgba(148,163,184,0.3); border-left: 3px solid #64748B; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 16px; font-weight: 700; letter-spacing: 3px; text-transform: uppercase; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);"
                        onclick={on_quit}
                    >
                        {"Quit to Title"}
                    </button>
                </div>
            </div>
        </div>
    }
}

fn render_stats_table(m: &MatchUpdate) -> Html {
    if m.players.is_empty() {
        return html! {
            <div style="color: #94A3B8; font-size: 12px; text-align: center; padding: 16px;">
                {"(no player stats)"}
            </div>
        };
    }

    html! {
        <div style="max-height: 340px; overflow-y: auto;">
            <table style="width: 100%; border-collapse: collapse; font-size: 12px; letter-spacing: 1px;">
                <thead>
                    <tr style="color: #64748B; text-transform: uppercase; border-bottom: 1px solid rgba(148,163,184,0.2);">
                        <th style="text-align: left; padding: 8px 6px; font-weight: 700;">{"#"}</th>
                        <th style="text-align: left; padding: 8px 6px; font-weight: 700;">{"Name"}</th>
                        <th style="text-align: left; padding: 8px 6px; font-weight: 700;">{"Team"}</th>
                        <th style="text-align: left; padding: 8px 6px; font-weight: 700;">{"Ship"}</th>
                        <th style="text-align: right; padding: 8px 6px; font-weight: 700;">{"K"}</th>
                        <th style="text-align: right; padding: 8px 6px; font-weight: 700;">{"C"}</th>
                        <th style="text-align: right; padding: 8px 6px; font-weight: 700;">{"Pts"}</th>
                    </tr>
                </thead>
                <tbody>
                    { for m.players.iter().enumerate().map(|(i, p)| render_stats_row(i, p)) }
                </tbody>
            </table>
        </div>
    }
}

fn render_stats_row(rank: usize, p: &common::protocol::PlayerMatchStatsDto) -> Html {
    let team_color = match p.team {
        MatchTeam::Blue => "#60A5FA",
        MatchTeam::Red => "#F87171",
    };
    let team_label = match p.team {
        MatchTeam::Blue => "BLUE",
        MatchTeam::Red => "RED",
    };
    let name_color = if p.is_you { "#FCD34D" } else { "#E2E8F0" };
    let row_bg = if p.is_you {
        "rgba(252,211,77,0.08)"
    } else {
        "transparent"
    };
    let row_style = format!(
        "background: {}; border-bottom: 1px solid rgba(148,163,184,0.1);",
        row_bg
    );

    let alias_str = p.alias.as_str();
    let display_name: String = if p.is_you {
        format!("{} (YOU)", alias_str)
    } else {
        alias_str.to_string()
    };

    let ship_label = match p.ship {
        Some(entity_type) => format!("{:?}", entity_type),
        None => "—".to_string(),
    };

    html! {
        <tr style={row_style}>
            <td style="padding: 8px 6px; color: #94A3B8; font-weight: 700;">{format!("{}", rank + 1)}</td>
            <td style={format!("padding: 8px 6px; color: {}; font-weight: 700;", name_color)}>{display_name}</td>
            <td style={format!("padding: 8px 6px; color: {}; font-weight: 700;", team_color)}>{team_label}</td>
            <td style="padding: 8px 6px; color: #94A3B8;">{ship_label}</td>
            <td style="padding: 8px 6px; text-align: right; color: #E2E8F0;">{format!("{}", p.kills)}</td>
            <td style="padding: 8px 6px; text-align: right; color: #E2E8F0;">{format!("{}", p.captures)}</td>
            <td style="padding: 8px 6px; text-align: right; color: #E2E8F0; font-weight: 700;">{format!("{}", p.personal_points)}</td>
        </tr>
    }
}
