// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use yew::{html, Html};

pub fn logo() -> Html {
    html! {
        <div style="
            text-align: center;
            width: 100%;
            padding: 1rem 0;
        ">
            <h1 style="
                font-family: 'Black Ops One', 'Menlo', monospace;
                font-size: 104px;
                font-weight: 400;
                color: #4ADE80;
                letter-spacing: 4px;
                text-transform: uppercase;
                text-shadow: 0 0 24px rgba(74,222,128,0.3), 0 2px 0 rgba(0,0,0,0.5);
                margin: 0;
            ">{"WARSHIPS"}</h1>
            <p style="
                font-family: 'Menlo', 'SF Mono', 'Courier New', monospace;
                font-size: 14px;
                color: #94A3B8;
                letter-spacing: 2px;
                text-transform: uppercase;
                margin-top: 8px;
            ">{"NAVAL COMBAT"}</p>
        </div>
    }
}
