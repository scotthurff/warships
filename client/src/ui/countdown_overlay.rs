// SPDX-FileCopyrightText: 2026 scotthurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Full-screen "3 / 2 / 1 / FIGHT" overlay rendered during
//! `MatchPhase::Countdown`. Hidden once the server transitions to
//! `MatchPhase::Playing`. The overlay reads its number directly from
//! `MatchUpdate.remaining_ms` so it stays in sync with server time.

use common::protocol::MatchUpdate;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct CountdownOverlayProps {
    pub match_update: MatchUpdate,
}

#[function_component(CountdownOverlay)]
pub fn countdown_overlay(props: &CountdownOverlayProps) -> Html {
    // Round up so the last fraction of a second reads "1" not "0".
    let seconds = (props.match_update.remaining_ms + 999) / 1000;
    let label = match seconds {
        0 => "FIGHT".to_string(),
        s => s.to_string(),
    };

    html! {
        <div style="position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; pointer-events: none; background: radial-gradient(ellipse at center, rgba(15,23,42,0.35) 0%, rgba(15,23,42,0.15) 100%);">
            <div
                key={seconds}
                style="font-family: 'Black Ops One', 'Menlo', monospace; font-size: 14rem; font-weight: 900; color: #FCD34D; text-shadow: 0 0 40px rgba(252,211,77,0.6), 0 4px 12px rgba(0,0,0,0.8); letter-spacing: 8px; animation: cdpulse 1s ease-out;"
            >
                {label}
            </div>
            <style>
                {"@keyframes cdpulse { 0% { transform: scale(0.7); opacity: 0; } 30% { transform: scale(1.15); opacity: 1; } 100% { transform: scale(1); opacity: 0.85; } }"}
            </style>
        </div>
    }
}
