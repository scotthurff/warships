// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use stylist::yew::styled_component;
use yew::{html, Html};

pub fn logo() -> Html {
    html! {
        <div style="
            text-align: center;
            width: 100%;
            padding: 1rem 0;
        ">
            <h1 style="
                font-family: 'Black Ops One', monospace, sans-serif;
                font-size: 72px;
                font-weight: 400;
                color: #c8dce8;
                text-shadow: 0 0 40px rgba(100,160,220,0.4), 0 2px 8px rgba(0,0,0,0.6);
                letter-spacing: 8px;
                margin: 0;
                text-transform: uppercase;
            ">{"WARSHIPS"}</h1>
            <div style="
                font-family: system-ui, sans-serif;
                color: rgba(180,200,220,0.6);
                font-size: 14px;
                letter-spacing: 6px;
                text-transform: uppercase;
                margin-top: 4px;
            ">{"NAVAL COMBAT"}</div>
        </div>
    }
}
