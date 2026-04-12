// SPDX-FileCopyrightText: 2026 scotthurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Full-screen match results interstitial.
//!
//! Rendered when `MatchPhase::Ended { winner }` fires. Takes over the
//! entire viewport with a heavy blur backdrop so the player can't miss
//! it. Action buttons ("Play Again" / "Quit to Title") are positioned
//! in the **top-right** corner — deliberately far from the bottom-
//! left/right touch controls — and start **disabled for 1.5 s** so the
//! last rapid-fire taps from gameplay can't accidentally dismiss the
//! screen.

use crate::ui::UiEvent;
use crate::Mk48Game;
use common::protocol::{MatchTeam, MatchUpdate, MatchWinner};
use gloo_timers::callback::Timeout;
use kodiak_client::use_ui_event_callback;
use yew::prelude::*;

/// How long buttons stay disabled after the overlay appears (ms).
const BUTTON_DELAY_MS: u32 = 1500;

#[derive(Properties, PartialEq)]
pub struct MatchEndOverlayProps {
    pub match_update: MatchUpdate,
    pub winner: MatchWinner,
}

#[function_component(MatchEndOverlay)]
pub fn match_end_overlay(props: &MatchEndOverlayProps) -> Html {
    let ui_event_callback = use_ui_event_callback::<Mk48Game>();

    // ── Button activation delay ──────────────────────────────────
    // Starts false, flips to true after BUTTON_DELAY_MS. Prevents
    // stray taps from the last moments of gameplay from immediately
    // dismissing the results screen.
    let buttons_active = use_state(|| false);
    {
        let active = buttons_active.clone();
        use_effect_with((), move |_| {
            let timeout = Timeout::new(BUTTON_DELAY_MS, move || active.set(true));
            move || drop(timeout)
        });
    }

    let on_play_again = {
        let cb = ui_event_callback.clone();
        let active = buttons_active.clone();
        Callback::from(move |_: MouseEvent| {
            if *active { cb.emit(UiEvent::PlayAgain); }
        })
    };
    let on_quit = {
        let cb = ui_event_callback.clone();
        let active = buttons_active.clone();
        Callback::from(move |_: MouseEvent| {
            if *active { cb.emit(UiEvent::QuitToTitle); }
        })
    };

    let is_active = *buttons_active;

    let (winner_label, winner_color, winner_glow) = match props.winner {
        MatchWinner::Blue => ("BLUE TEAM WINS", "#60A5FA", "rgba(96,165,250,0.4)"),
        MatchWinner::Red => ("RED TEAM WINS", "#F87171", "rgba(248,113,113,0.4)"),
        MatchWinner::Draw => ("DRAW", "#FCD34D", "rgba(252,211,77,0.4)"),
    };

    let m = &props.match_update;

    // Button opacity: dim when inactive, full when active
    let btn_opacity = if is_active { "1" } else { "0.35" };
    let btn_cursor = if is_active { "pointer" } else { "default" };

    html! {
        // ── Full-screen backdrop ─────────────────────────────────
        <div style="
            position: fixed; inset: 0; z-index: 9999;
            display: flex; flex-direction: column;
            background: rgba(15,23,42,0.92);
            backdrop-filter: blur(12px);
            -webkit-backdrop-filter: blur(12px);
            font-family: 'Menlo', 'SF Mono', 'Courier New', monospace;
            overflow-y: auto;
        ">
            // ── Action buttons — top-right ───────────────────────
            // Positioned away from bottom touch controls so rapid
            // FIRE/TORP taps can't accidentally dismiss the screen.
            <div style="
                position: absolute; top: 24px; right: 24px;
                display: flex; gap: 12px; z-index: 10001;
            ">
                <button
                    style={format!("
                        display: flex; align-items: center; justify-content: center;
                        min-width: 160px; height: 48px; padding: 0 28px;
                        background: rgba(15,23,42,0.95);
                        color: #4ADE80;
                        border: 1px solid rgba(34,197,94,0.4);
                        border-left: 3px solid #22C55E;
                        border-radius: 2px;
                        font-family: 'Menlo', 'SF Mono', 'Courier New', monospace;
                        font-size: 14px; font-weight: 700;
                        letter-spacing: 2px; text-transform: uppercase;
                        cursor: {}; opacity: {};
                        box-shadow: 0 2px 8px rgba(0,0,0,0.5);
                        transition: opacity 0.4s ease;
                    ", btn_cursor, btn_opacity)}
                    onclick={on_play_again}
                >
                    {"Play Again"}
                </button>
                <button
                    style={format!("
                        display: flex; align-items: center; justify-content: center;
                        min-width: 160px; height: 48px; padding: 0 28px;
                        background: rgba(15,23,42,0.95);
                        color: #94A3B8;
                        border: 1px solid rgba(148,163,184,0.3);
                        border-left: 3px solid #64748B;
                        border-radius: 2px;
                        font-family: 'Menlo', 'SF Mono', 'Courier New', monospace;
                        font-size: 14px; font-weight: 700;
                        letter-spacing: 2px; text-transform: uppercase;
                        cursor: {}; opacity: {};
                        box-shadow: 0 2px 8px rgba(0,0,0,0.5);
                        transition: opacity 0.4s ease;
                    ", btn_cursor, btn_opacity)}
                    onclick={on_quit}
                >
                    {"Quit to Title"}
                </button>
            </div>

            // ── Main content — vertically centered ───────────────
            <div style="
                flex: 1; display: flex; flex-direction: column;
                align-items: center; justify-content: center;
                padding: 80px 40px 40px;
                gap: 32px;
            ">
                // ── Winner banner ────────────────────────────────
                <div style={format!("
                    color: {};
                    font-family: 'Black Ops One', 'Menlo', monospace;
                    font-size: clamp(36px, 8vw, 72px);
                    font-weight: 900;
                    letter-spacing: 8px;
                    text-align: center;
                    text-shadow: 0 0 60px {}, 0 4px 16px rgba(0,0,0,0.8);
                    animation: winPulse 2s ease-in-out infinite;
                ", winner_color, winner_glow)}>
                    {winner_label}
                </div>

                // ── MATCH COMPLETE subheading ────────────────────
                <div style="
                    color: #64748B;
                    font-size: 13px; font-weight: 700;
                    letter-spacing: 4px; text-transform: uppercase;
                ">
                    {"MATCH COMPLETE"}
                </div>

                // ── Final scores ─────────────────────────────────
                <div style="
                    display: flex; align-items: center; justify-content: center;
                    gap: 48px; font-size: 28px; font-weight: 700;
                    letter-spacing: 4px;
                    padding: 20px 0;
                    border-top: 1px solid rgba(148,163,184,0.15);
                    border-bottom: 1px solid rgba(148,163,184,0.15);
                    width: min(640px, 90vw);
                ">
                    <div style="display: flex; flex-direction: column; align-items: center; gap: 4px;">
                        <div style="color: #64748B; font-size: 11px; letter-spacing: 3px;">{"BLUE"}</div>
                        <div style="color: #60A5FA;">{format!("{}", m.blue_score)}</div>
                    </div>
                    <div style="color: #334155; font-size: 20px;">{"—"}</div>
                    <div style="display: flex; flex-direction: column; align-items: center; gap: 4px;">
                        <div style="color: #64748B; font-size: 11px; letter-spacing: 3px;">{"RED"}</div>
                        <div style="color: #F87171;">{format!("{}", m.red_score)}</div>
                    </div>
                </div>

                // ── Stats table ──────────────────────────────────
                <div style="
                    width: min(720px, 90vw);
                    background: rgba(15,23,42,0.6);
                    border: 1px solid rgba(148,163,184,0.15);
                    border-radius: 2px;
                    padding: 16px;
                ">
                    { render_stats_table(m) }
                </div>

                // ── Activation hint ──────────────────────────────
                if !is_active {
                    <div style="
                        color: #475569; font-size: 11px;
                        letter-spacing: 2px; text-transform: uppercase;
                        animation: fadeInHint 0.5s ease-in;
                    ">
                        {"Review your stats..."}
                    </div>
                }
            </div>

            // ── Animations ───────────────────────────────────────
            <style>
                {"
                    @keyframes winPulse {
                        0%, 100% { transform: scale(1); }
                        50% { transform: scale(1.03); }
                    }
                    @keyframes fadeInHint {
                        from { opacity: 0; }
                        to { opacity: 1; }
                    }
                "}
            </style>
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
        <div style="max-height: 360px; overflow-y: auto;">
            <table style="width: 100%; border-collapse: collapse; font-size: 13px; letter-spacing: 1px;">
                <thead>
                    <tr style="color: #475569; text-transform: uppercase; border-bottom: 1px solid rgba(148,163,184,0.2);">
                        <th style="text-align: left; padding: 10px 8px; font-weight: 700; font-size: 10px;">{"#"}</th>
                        <th style="text-align: left; padding: 10px 8px; font-weight: 700; font-size: 10px;">{"Name"}</th>
                        <th style="text-align: left; padding: 10px 8px; font-weight: 700; font-size: 10px;">{"Team"}</th>
                        <th style="text-align: left; padding: 10px 8px; font-weight: 700; font-size: 10px;">{"Ship"}</th>
                        <th style="text-align: right; padding: 10px 8px; font-weight: 700; font-size: 10px;">{"Kills"}</th>
                        <th style="text-align: right; padding: 10px 8px; font-weight: 700; font-size: 10px;">{"Captures"}</th>
                        <th style="text-align: right; padding: 10px 8px; font-weight: 700; font-size: 10px;">{"Points"}</th>
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
        "rgba(252,211,77,0.06)"
    } else if rank % 2 == 0 {
        "rgba(148,163,184,0.03)"
    } else {
        "transparent"
    };
    let row_style = format!(
        "background: {}; border-bottom: 1px solid rgba(148,163,184,0.08);",
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
            <td style="padding: 10px 8px; color: #64748B; font-weight: 700;">{format!("{}", rank + 1)}</td>
            <td style={format!("padding: 10px 8px; color: {}; font-weight: 700;", name_color)}>{display_name}</td>
            <td style={format!("padding: 10px 8px; color: {}; font-weight: 700; font-size: 11px;", team_color)}>{team_label}</td>
            <td style="padding: 10px 8px; color: #94A3B8; font-size: 12px;">{ship_label}</td>
            <td style="padding: 10px 8px; text-align: right; color: #E2E8F0;">{format!("{}", p.kills)}</td>
            <td style="padding: 10px 8px; text-align: right; color: #E2E8F0;">{format!("{}", p.captures)}</td>
            <td style="padding: 10px 8px; text-align: right; color: #E2E8F0; font-weight: 700;">{format!("{}", p.personal_points)}</td>
        </tr>
    }
}
