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

    // Wargame military panel style
    let btn_base = "
        font-family: 'Menlo', 'SF Mono', 'Courier New', monospace;
        font-size: 13px;
        font-weight: 700;
        letter-spacing: 2px;
        text-transform: uppercase;
        color: #E2E8F0;
        background: rgba(15,23,42,0.92);
        border: 1px solid rgba(100,116,139,0.4);
        border-radius: 2px;
        cursor: pointer;
        user-select: none;
        -webkit-user-select: none;
        touch-action: manipulation;
        box-shadow: 0 2px 8px rgba(0,0,0,0.5);
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

    // ── Fire buttons ──
    let on_fire = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchFire); })
    };
    let on_torpedo = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchTorpedo); })
    };

    // ── Zoom buttons ──
    let on_zoom_in = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchZoom(-2.0)); })
    };
    let on_zoom_out = {
        let cb = ui_event_callback.clone();
        Callback::from(move |_: MouseEvent| { cb.emit(UiEvent::TouchZoom(2.0)); })
    };

    // Track active speed for highlighting: 0=rev, 1=stop, 2=1/4, 3=1/2, 4=full
    let active_speed = use_state(|| 1u8);
    let on_speed_rev = {
        let cb = ui_event_callback.clone();
        let active = active_speed.clone();
        Callback::from(move |_: MouseEvent| { active.set(0); cb.emit(UiEvent::TouchThrottle(-0.25)); })
    };
    let on_speed_stop = {
        let cb = ui_event_callback.clone();
        let active = active_speed.clone();
        Callback::from(move |_: MouseEvent| { active.set(1); cb.emit(UiEvent::TouchThrottle(0.0)); })
    };
    let on_speed_quarter = {
        let cb = ui_event_callback.clone();
        let active = active_speed.clone();
        Callback::from(move |_: MouseEvent| { active.set(2); cb.emit(UiEvent::TouchThrottle(0.25)); })
    };
    let on_speed_half = {
        let cb = ui_event_callback.clone();
        let active = active_speed.clone();
        Callback::from(move |_: MouseEvent| { active.set(3); cb.emit(UiEvent::TouchThrottle(0.5)); })
    };
    let on_speed_full = {
        let cb = ui_event_callback.clone();
        let active = active_speed.clone();
        Callback::from(move |_: MouseEvent| { active.set(4); cb.emit(UiEvent::TouchThrottle(1.0)); })
    };

    let speed_val = *active_speed;
    let speed_bg = |idx: u8, accent: &str, border_accent: &str| -> String {
        if speed_val == idx {
            format!("{btn_base} width: 48px; height: 44px; font-size: 11px; color: {accent}; border-left: 3px solid {border_accent};")
        } else {
            format!("{btn_base} width: 48px; height: 44px; font-size: 11px; color: #64748B;")
        }
    };

    html! {
        <>
            // Navigation controls — bottom left, wargame military panel style
            <Positioner id="touch_nav" position={Position::BottomLeft{margin: "1rem"}}>
                <div style="display: flex; flex-direction: column; gap: 6px; align-items: center;">
                    // Speed buttons (REV / STOP / 1/4 / 1/2 / FULL)
                    <div style="display: flex; gap: 4px;">
                        <button
                            style={speed_bg(0, "#F87171", "#EF4444")}
                            onclick={on_speed_rev}
                        >{"REV"}</button>
                        <button
                            style={speed_bg(1, "#FCA5A5", "#EF4444")}
                            onclick={on_speed_stop}
                        >{"STOP"}</button>
                        <button
                            style={speed_bg(2, "#60A5FA", "#3B82F6")}
                            onclick={on_speed_quarter}
                        >{"1/4"}</button>
                        <button
                            style={speed_bg(3, "#60A5FA", "#3B82F6")}
                            onclick={on_speed_half}
                        >{"1/2"}</button>
                        <button
                            style={speed_bg(4, "#4ADE80", "#22C55E")}
                            onclick={on_speed_full}
                        >{"FULL"}</button>
                    </div>
                    // Rudder buttons — wargame blue accent
                    <div style="display: flex; gap: 4px;">
                        <button
                            style={format!("{btn_base} width: 80px; height: 56px; font-size: 22px; color: #60A5FA; border-left: 3px solid #3B82F6;")}
                            onpointerdown={on_rudder_left}
                            onpointerup={on_rudder_center.clone()}
                            onpointerleave={on_rudder_center.clone()}
                        >{"◀"}</button>
                        <button
                            style={format!("{btn_base} width: 80px; height: 56px; font-size: 22px; color: #60A5FA; border-right: 3px solid #3B82F6;")}
                            onpointerdown={on_rudder_right}
                            onpointerup={on_rudder_center.clone()}
                            onpointerleave={on_rudder_center}
                        >{"▶"}</button>
                    </div>
                </div>
            </Positioner>

            // Fire buttons — bottom right, wargame attack style
            <Positioner id="touch_fire" position={Position::BottomRight{margin: "1rem"}}>
                <div style="display: flex; flex-direction: column; gap: 6px; align-items: center;">
                    <button
                        style={format!("{btn_base} width: 64px; height: 56px; color: #60A5FA; border-left: 3px solid #3B82F6;")}
                        onclick={on_torpedo}
                    >{"TORP"}</button>
                    <button
                        style={format!("{btn_base} width: 80px; height: 64px; color: #FCA5A5; border-left: 3px solid #EF4444; font-size: 16px;")}
                        onclick={on_fire}
                    >{"FIRE"}</button>
                </div>
            </Positioner>

            // Zoom buttons — top right, wargame muted style
            <Positioner id="touch_zoom" position={Position::TopRight{margin: "1rem"}}>
                <div style="display: flex; gap: 4px;">
                    <button
                        style={format!("{btn_base} width: 48px; height: 48px; font-size: 18px; color: #94A3B8;")}
                        onclick={on_zoom_in}
                    >{"+"}</button>
                    <button
                        style={format!("{btn_base} width: 48px; height: 48px; font-size: 18px; color: #94A3B8;")}
                        onclick={on_zoom_out}
                    >{"-"}</button>
                </div>
            </Positioner>
        </>
    }
}
