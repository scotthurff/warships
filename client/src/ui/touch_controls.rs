// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::game::Mk48Game;
use crate::ui::game_ui::UiEvent;
use kodiak_client::{use_ui_event_callback, Position, Positioner};
use stylist::yew::styled_component;
use yew::prelude::*;

#[styled_component(TouchControls)]
pub fn touch_controls() -> Html {
    let ui_event_callback = use_ui_event_callback::<Mk48Game>();

    let btn_base = "
        font-family: 'Menlo', 'SF Mono', 'Courier New', monospace;
        font-size: 0.85rem;
        font-weight: bold;
        letter-spacing: 1px;
        color: white;
        border: 2px solid rgba(255,255,255,0.3);
        border-radius: 12px;
        cursor: pointer;
        user-select: none;
        -webkit-user-select: none;
        touch-action: manipulation;
        backdrop-filter: blur(4px);
        -webkit-backdrop-filter: blur(4px);
        display: flex;
        align-items: center;
        justify-content: center;
    ";

    // ── Rudder buttons (bottom-left) ──
    let on_rudder_left = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: PointerEvent| { cb.emit(UiEvent::TouchRudder(1.0)); })
    };
    let on_rudder_center = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: PointerEvent| { cb.emit(UiEvent::TouchRudder(0.0)); })
    };
    let on_rudder_right = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: PointerEvent| { cb.emit(UiEvent::TouchRudder(-1.0)); })
    };

    // ── Speed buttons ──
    let on_speed_stop = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchThrottle(0.0)); })
    };
    let on_speed_half = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchThrottle(0.5)); })
    };
    let on_speed_full = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchThrottle(1.0)); })
    };

    // ── Fire button ──
    let on_fire = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchFire); })
    };

    html! {
        <>
            // Navigation controls — bottom left
            <Positioner id="touch_nav" position={Position::BottomLeft{margin: "1rem"}}>
                <div style="display: flex; flex-direction: column; gap: 0.5rem; align-items: center;">
                    // Speed buttons
                    <div style="display: flex; gap: 0.4rem;">
                        <button
                            style={format!("{btn_base} width: 56px; height: 44px; background: rgba(200,50,50,0.4);")}
                            onclick={on_speed_stop}
                        >{"STOP"}</button>
                        <button
                            style={format!("{btn_base} width: 56px; height: 44px; background: rgba(50,100,200,0.4);")}
                            onclick={on_speed_half}
                        >{"1/2"}</button>
                        <button
                            style={format!("{btn_base} width: 56px; height: 44px; background: rgba(50,180,80,0.4);")}
                            onclick={on_speed_full}
                        >{"FULL"}</button>
                    </div>
                    // Rudder buttons
                    <div style="display: flex; gap: 0.4rem;">
                        <button
                            style={format!("{btn_base} width: 80px; height: 64px; font-size: 1.8rem; background: rgba(0,0,0,0.35);")}
                            onpointerdown={on_rudder_left}
                            onpointerup={on_rudder_center.clone()}
                            onpointerleave={on_rudder_center.clone()}
                        >{"◀"}</button>
                        <button
                            style={format!("{btn_base} width: 80px; height: 64px; font-size: 1.8rem; background: rgba(0,0,0,0.35);")}
                            onpointerdown={on_rudder_right}
                            onpointerup={on_rudder_center.clone()}
                            onpointerleave={on_rudder_center}
                        >{"▶"}</button>
                    </div>
                </div>
            </Positioner>

            // Fire button — bottom right
            <Positioner id="touch_fire" position={Position::BottomRight{margin: "1rem"}}>
                <button
                    style={format!("{btn_base} width: 80px; height: 80px; border-radius: 50%; background: radial-gradient(circle at 40% 35%, #ff4444, #aa1111); border: 3px solid rgba(255,100,80,0.6); box-shadow: 0 0 20px rgba(255,60,40,0.3); font-size: 1rem;")}
                    onclick={on_fire}
                >{"FIRE"}</button>
            </Positioner>
        </>
    }
}
