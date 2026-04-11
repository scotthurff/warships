// SPDX-FileCopyrightText: 2026 scotthurff
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Respawn overlay used during Capture the Area mode. Shows a short
//! "RESPAWNING..." countdown and auto-fires `UiEvent::Respawn(ship)` after
//! 1500ms. No ship picker — the player already chose their loadout at
//! match start and the server will place them back at their team base.

use crate::ui::UiEvent;
use crate::Mk48Game;
use common::entity::EntityType;
use gloo_timers::callback::Interval;
use kodiak_client::use_ui_event_callback;
use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct CtaRespawnOverlayProps {
    /// The ship to respawn as. If `None`, the overlay renders a
    /// message but cannot trigger the respawn — the player has to
    /// intervene. (Should not happen under normal gameplay.)
    pub ship: Option<EntityType>,
}

#[function_component(CtaRespawnOverlay)]
pub fn cta_respawn_overlay(props: &CtaRespawnOverlayProps) -> Html {
    let ui_event_callback = use_ui_event_callback::<Mk48Game>();
    let ship = props.ship;

    // Schedule a recurring Interval that fires UiEvent::Respawn every
    // 1500ms while the overlay is mounted. A one-shot Timeout was the
    // previous implementation, but if the first Spawn command failed
    // server-side (terrain blocks the slot, another ship in the way,
    // etc) the player got stuck on the respawn screen forever with no
    // retry. The interval keeps hammering until the spawn succeeds,
    // at which point the status transitions Respawning → Playing, the
    // overlay unmounts, and Interval's Drop cancels it automatically.
    use_effect_with(ship, move |_| {
        let cb = ui_event_callback.clone();
        let interval = ship.map(move |entity_type| {
            Interval::new(1500, move || {
                cb.emit(UiEvent::Respawn(entity_type));
            })
        });
        // Dropping the Interval on unmount cancels it.
        move || drop(interval)
    });

    html! {
        <div style="position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; pointer-events: none; background: rgba(15, 23, 42, 0.55); animation: ctafade 0.3s ease-out;">
            <div style="display: flex; flex-direction: column; align-items: center; gap: 16px; padding: 32px 56px; background: rgba(15, 23, 42, 0.92); border: 1px solid rgba(148, 163, 184, 0.4); border-left: 3px solid #FCD34D; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; box-shadow: 0 4px 16px rgba(0, 0, 0, 0.6);">
                <div style="color: #FCD34D; font-size: 22px; font-weight: 700; letter-spacing: 4px; text-transform: uppercase;">{"Respawning"}</div>
                <div style="color: #94A3B8; font-size: 13px; font-weight: 400; letter-spacing: 2px; text-transform: uppercase;">{"Returning to base"}</div>
            </div>
        </div>
    }
}
