// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::game::Mk48Game;
use crate::ui::about_dialog::AboutDialog;
use crate::ui::countdown_overlay::CountdownOverlay;
use crate::ui::cta_respawn_overlay::CtaRespawnOverlay;
use crate::ui::help_dialog::HelpDialog;
use crate::ui::hint::Hint;
use crate::ui::logo::logo;
use crate::ui::match_end_overlay::MatchEndOverlay;
use crate::ui::minimap::{Minimap, MinimapEntry};
use crate::ui::references_dialog::ReferencesDialog;
use crate::ui::respawn_overlay::RespawnOverlay;
use crate::ui::ship_picker::ShipPicker;
use crate::ui::ships_dialog::ShipsDialog;
use crate::ui::status_overlay::StatusOverlay;
use crate::ui::touch_controls::TouchControls;
use crate::ui::upgrade_overlay::UpgradeOverlay;
use crate::ui::Mk48Phrases;
use common::altitude::Altitude;
use common::angle::Angle;
use common::death_reason::DeathReason;
use common::entity::{EntityId, EntityType};
use common::protocol::{GameMode, MatchUpdate, TeamDto, TeamRequest};
use common::velocity::Velocity;
use kodiak_client::glam::Vec2;
use kodiak_client::yew_router::Routable;
use kodiak_client::{
    translate, use_ctw,
    use_gctw, ClientContext, GameClient, Instruction, PathParam,
    PlayerAlias, PlayerId, Position, Positioner, PropertiesWrapper, RoutableExt, SmolRoutable,
    TeamId, Translator,
};
use std::collections::HashMap;
use stylist::yew::styled_component;
use yew::prelude::*;

/// Which step of the title-screen flow the player is currently on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TitleStep {
    /// Mode selector (Free Roam vs Capture the Area) + Continue button.
    ModeSelect,
    /// Ship picker with level tabs, stat detail, and Start Game button.
    /// Both modes go through this — in Free Roam the picked ship's level
    /// seeds the player's starting score (see server/src/world_inbound.rs
    /// Spawn::apply). In CTA it picks the match loadout.
    ShipSelect,
}

/// The default player name used on the title screen. The upstream default
/// rendered as "Guest" which felt impersonal for a single-player game.
fn default_alias() -> PlayerAlias {
    PlayerAlias::new_unsanitized("Admiral")
}

#[styled_component(Mk48Ui)]
pub fn mk48_ui(props: &PropertiesWrapper<UiProps>) -> Html {
    let ctw = use_ctw();
    let nexus = ctw.escaping.is_escaping();
    let gctw = use_gctw::<Mk48Game>();

    // Currently-selected game mode on the title screen. Defaults to Free Roam.
    // Persists across re-renders of this component.
    let selected_mode = use_state(|| GameMode::FreeRoam);

    // Currently-selected ship on the title screen. Required in both
    // modes — the ship propagates into the Spawn command's entity_type.
    // In Free Roam it also seeds the starting score; in CTA it's the
    // match loadout.
    let selected_ship = use_state::<Option<EntityType>, _>(|| None);

    // Which step of the title flow we're on. Both modes go through
    // both steps: ModeSelect picks the mode, ShipSelect picks the ship.
    let title_step = use_state(|| TitleStep::ModeSelect);

    // Currently-selected difficulty. Seeded from the atomic global
    // (which is the server-side source of truth) so reopening the
    // title screen reflects the last choice. Each on_select_* handler
    // below updates both this cell (for paint) and the global (for bots).
    let selected_difficulty = use_state(common::Difficulty::get_global);

    // Reset the three title-screen cells when the user LEAVES a
    // match (match_update transitions from Some back to None). Fires
    // on Quit to Title so the next return to title lands on
    // ModeSelect with no stale pre-selected ship.
    //
    // Does NOT fire when entering a match (None → Some). That was a
    // regression: clicking Start Game bumps match_update from None
    // to Some(new match_id), which fired the old dep and reset
    // title_step to ModeSelect mid-transition — before UiStatus had
    // moved from Spawning to Playing. The user briefly saw
    // ModeSelect chrome overlaying gameplay.
    //
    // Play Again (match_update stays Some, match_id bumps) also no
    // longer fires. Fine — the user is in-match during Play Again
    // so title state is invisible; it gets reset on the next
    // actual Quit-to-Title anyway.
    {
        let selected_mode = selected_mode.clone();
        let selected_ship = selected_ship.clone();
        let title_step = title_step.clone();
        let in_match = props.match_update.is_some();
        use_effect_with(in_match, move |currently_in_match| {
            if !*currently_in_match {
                selected_mode.set(GameMode::FreeRoam);
                selected_ship.set(None);
                title_step.set(TitleStep::ModeSelect);
            }
            || ()
        });
    }

    let on_play = {
        let mode = *selected_mode;
        let selected_ship = selected_ship.clone();
        gctw.send_ui_event_callback.reform(move |alias| UiEvent::Spawn {
            alias,
            entity_type: selected_ship.unwrap_or(EntityType::G5),
            game_mode: mode,
        })
    };

    let on_select_free_roam = {
        let selected_mode = selected_mode.clone();
        Callback::from(move |_: MouseEvent| selected_mode.set(GameMode::FreeRoam))
    };
    let on_select_cta = {
        let selected_mode = selected_mode.clone();
        Callback::from(move |_: MouseEvent| selected_mode.set(GameMode::CaptureTheArea))
    };
    let on_ship_pick = {
        let selected_ship = selected_ship.clone();
        Callback::from(move |entity_type: EntityType| selected_ship.set(Some(entity_type)))
    };
    // "Continue" from the mode-select step. Both modes advance to the
    // ship picker — in Free Roam the picked ship determines the starting
    // level (Spawn::apply seeds player.score to level_to_score(ship.level)
    // for title-screen spawns). See plans/freeroam-ship-picker.md.
    let on_continue = {
        let title_step = title_step.clone();
        Callback::from(move |_: MouseEvent| {
            title_step.set(TitleStep::ShipSelect);
        })
    };
    // "Back" from the ship-select step.
    let on_back_to_modes = {
        let title_step = title_step.clone();
        Callback::from(move |_: MouseEvent| {
            title_step.set(TitleStep::ModeSelect);
        })
    };
    // "Start Game" from the ship-select step.
    let on_start_from_picker = {
        let play_cb = on_play.clone();
        Callback::from(move |_: MouseEvent| {
            play_cb.emit(default_alias());
        })
    };

    let margin = "0.5rem";
    let status = props.status.clone();

    // Mode-selector tile styling (computed outside html! since Yew's macro
    // doesn't allow bare `let` statements inside its JSX block).
    let free_selected = *selected_mode == GameMode::FreeRoam;
    let cta_selected = *selected_mode == GameMode::CaptureTheArea;
    let tile_base = "display: flex; flex-direction: column; align-items: center; justify-content: center; width: 220px; height: 140px; padding: 20px; background: rgba(15,23,42,0.92); border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);";
    let free_style = if free_selected {
        format!("{} color: #4ADE80; border: 2px solid #22C55E; border-left: 4px solid #22C55E;", tile_base)
    } else {
        format!("{} color: #94A3B8; border: 1px solid rgba(148,163,184,0.3); border-left: 3px solid #64748B;", tile_base)
    };
    let cta_style = if cta_selected {
        format!("{} color: #FCD34D; border: 2px solid #EAB308; border-left: 4px solid #EAB308;", tile_base)
    } else {
        format!("{} color: #94A3B8; border: 1px solid rgba(148,163,184,0.3); border-left: 3px solid #64748B;", tile_base)
    };

    // Difficulty buttons — mirror the mode-tile selected/unselected
    // pattern so it's obvious which difficulty is active. Selected: full
    // accent color + 2px border + 4px left stripe. Unselected: muted
    // slate + thinner borders. Colors tuned per-difficulty to match the
    // existing palette (green = Captain/easy, yellow = Admiral/medium,
    // red = Fleet Cmdr/hard).
    let diff_btn_base = "display: flex; align-items: center; justify-content: center; min-width: 140px; height: 48px; padding: 0 28px; background: rgba(15,23,42,0.92); border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 14px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);";
    let diff_unselected = "color: #94A3B8; border: 1px solid rgba(148,163,184,0.3); border-left: 3px solid #64748B;";
    let cap_selected_style = common::Difficulty::Captain == *selected_difficulty;
    let adm_selected_style = common::Difficulty::Admiral == *selected_difficulty;
    let fc_selected_style = common::Difficulty::FleetCommander == *selected_difficulty;
    let cap_style = if cap_selected_style {
        format!("{} color: #4ADE80; border: 2px solid #22C55E; border-left: 4px solid #22C55E;", diff_btn_base)
    } else {
        format!("{} {}", diff_btn_base, diff_unselected)
    };
    let adm_style = if adm_selected_style {
        format!("{} color: #FCD34D; border: 2px solid #EAB308; border-left: 4px solid #EAB308;", diff_btn_base)
    } else {
        format!("{} {}", diff_btn_base, diff_unselected)
    };
    let fc_style = if fc_selected_style {
        format!("{} color: #F87171; border: 2px solid #EF4444; border-left: 4px solid #EF4444;", diff_btn_base)
    } else {
        format!("{} {}", diff_btn_base, diff_unselected)
    };

    let on_select_captain = {
        let d = selected_difficulty.clone();
        Callback::from(move |_: MouseEvent| {
            common::Difficulty::set_global(common::Difficulty::Captain);
            d.set(common::Difficulty::Captain);
        })
    };
    let on_select_admiral = {
        let d = selected_difficulty.clone();
        Callback::from(move |_: MouseEvent| {
            common::Difficulty::set_global(common::Difficulty::Admiral);
            d.set(common::Difficulty::Admiral);
        })
    };
    let on_select_fleet_cmdr = {
        let d = selected_difficulty.clone();
        Callback::from(move |_: MouseEvent| {
            common::Difficulty::set_global(common::Difficulty::FleetCommander);
            d.set(common::Difficulty::FleetCommander);
        })
    };

    const SHOOT_HINT: &str = "First, select an available weapon. Then, click in the direction to fire. If you hold the click for too long, you won't shoot.";
    const HINTS: &[(&str, &[&str])] = &[
        ("Invitation links cannot currently be accepted by players that are already in game. They must send a join request instead.", &["/invite"]),
        ("If you are asking how you move, you click and hold to set your speed and direction (or use WASD).", &["how", "move"]),
        ("The controls are click and hold (or WASD) to move, click (or Space) to shoot.", &["how", "play"]),
        (SHOOT_HINT, &["how", "shoot"]),
        (SHOOT_HINT, &["how", "use weapons"]),
        (SHOOT_HINT, &["how", "fire"])
    ];

    // Countdown / match-end overlays live outside the Playing / Spawning
    // branches because they cross-cut phase state. The countdown fires
    // during MatchPhase::Countdown; the match-end results screen fires
    // during MatchPhase::Ended.
    let countdown_html = if let Some(m) = props.match_update.as_ref() {
        if m.phase == common::protocol::MatchPhase::Countdown {
            html! { <CountdownOverlay match_update={m.clone()} /> }
        } else {
            html! {}
        }
    } else {
        html! {}
    };
    let match_end_html = if let Some(m) = props.match_update.as_ref() {
        if let common::protocol::MatchPhase::Ended { winner } = m.phase {
            html! { <MatchEndOverlay match_update={m.clone()} {winner} /> }
        } else {
            html! {}
        }
    } else {
        html! {}
    };

    // Ship name labels as HTML overlays. Replaces kodiak's WebGL
    // bitmap-atlas text renderer (which pixellates at high zoom and
    // uses a fixed bitmap font). Positions come pre-projected from
    // the render loop in game.rs via Camera2d::to_client_position.
    //
    // `transform: translate(-50%, -100%)` centers each label
    // horizontally on its anchor and pushes it up so the anchor sits
    // at the bottom-center of the text — the ship position stays
    // below the floating name, matching the prior WebGL behavior.
    let ship_labels_html = {
        let labels = props.ship_labels.iter().map(|l| {
            let style = format!(
                "position: fixed; left: {}px; top: {}px; transform: translate(-50%, -100%); \
                 color: rgb({}, {}, {}); \
                 font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; \
                 font-size: 12px; font-weight: 700; \
                 letter-spacing: 1px; text-transform: uppercase; \
                 text-shadow: 0 1px 3px rgba(0,0,0,0.9), 0 0 8px rgba(0,0,0,0.6); \
                 pointer-events: none; white-space: nowrap; \
                 user-select: none; -webkit-user-select: none;",
                l.x, l.y, l.color[0], l.color[1], l.color[2]
            );
            html! { <div style={style}>{&l.alias}</div> }
        });
        html! { <>{ for labels }</> }
    };

    // Minimap: rendered whenever the match is active, regardless of
    // Playing vs Respawning vs Spawning status. Disappears on
    // `MatchPhase::Ended` since the results screen takes over.
    let minimap_html = if let Some(m) = props.match_update.as_ref() {
        if !matches!(m.phase, common::protocol::MatchPhase::Ended { .. }) {
            html! { <Minimap entries={props.minimap_entries.clone()} /> }
        } else {
            html! {}
        }
    } else {
        html! {}
    };

    html! {
        <>
            { ship_labels_html }
            { countdown_html }
            { match_end_html }
            { minimap_html }
            if matches!(status, UiStatus::Playing(_) | UiStatus::Respawning(_)) && !nexus {
                // Capture the Area HUD — timer + scores + capture bars, top-middle.
                // Only renders when the server is sending match updates
                // (i.e., the player is in CTA mode).
                if let Some(m) = props.match_update.as_ref() {
                    <Positioner id="match_hud" position={Position::TopMiddle{margin: "0.5rem"}}>
                        <div style="display: flex; flex-direction: column; align-items: stretch; gap: 8px; padding: 10px 18px; background: rgba(15,23,42,0.92); border: 1px solid rgba(148,163,184,0.4); border-left: 3px solid #4ADE80; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-weight: 700; letter-spacing: 2px; color: #E2E8F0; box-shadow: 0 2px 8px rgba(0,0,0,0.5); min-width: 320px;">
                            <div style="display: flex; align-items: center; justify-content: space-between; gap: 20px; font-size: 16px;">
                                <div style="color: #60A5FA;">{format!("BLUE {}", m.blue_score)}</div>
                                <div style="color: #FCD34D;">{format_match_clock(m)}</div>
                                <div style="color: #F87171;">{format!("{} RED", m.red_score)}</div>
                            </div>
                            { render_capture_bars(m) }
                        </div>
                    </Positioner>
                    // In-match Quit button — top-right corner, red
                    // danger styling so it's not accidentally tapped.
                    // Reuses the existing QuitToTitle command path
                    // (same handler as the match-end overlay button)
                    // so the teardown semantics are shared. Rendered
                    // here (inside the match_update.is_some() block)
                    // so it only appears during CTA matches.
                    <div style="position: fixed; top: 0.5rem; right: 0.5rem; z-index: 10;">
                        <button
                            style="display: flex; align-items: center; justify-content: center; min-width: 80px; height: 36px; padding: 0 14px; background: rgba(15,23,42,0.92); color: #F87171; border: 1px solid rgba(239,68,68,0.4); border-left: 3px solid #EF4444; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 12px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);"
                            onclick={gctw.send_ui_event_callback.reform(|_: MouseEvent| UiEvent::QuitToTitle)}
                        >
                            {"Quit"}
                        </button>
                    </div>
                }
                if let UiStatus::Playing(playing) = status {
                    <Positioner id="status" position={Position::BottomMiddle{margin: "0"}} max_width="45%">
                        <StatusOverlay
                            status={playing.clone()}
                            fps={gctw.settings_cache.fps_shown.then_some(props.fps)}
                        />
                    </Positioner>
                    // Hide the level-progress / upgrade overlay during
                    // Capture the Area — ships are fixed per match so
                    // there's no meaningful progression, and it clashes
                    // with the match HUD which lives in the same slot.
                    if props.match_update.is_none() {
                        <UpgradeOverlay
                            position={Position::TopMiddle{margin}}
                            status={playing.clone()}
                            score={props.score}
                        />
                    }
                    <Hint entity_type={playing.entity_type}/>
                    if props.touch_screen {
                        <TouchControls/>
                    }
                } else if let UiStatus::Respawning(respawning) = status {
                    if props.match_update.is_some() {
                        // Capture the Area: auto-respawn after 1.5s at team base,
                        // no ship picker.
                        <CtaRespawnOverlay ship={props.last_spawn_entity}/>
                    } else {
                        <RespawnOverlay status={respawning} score={props.score}/>
                    }
                }
            } else {
                if let UiStatus::Spawning = status {
                    <Positioner id="spawn" position={Position::Center}>
                        if *title_step == TitleStep::ModeSelect {
                            // ─── STEP 1: Mode select + difficulty ─────────────
                            <div style="display: flex; flex-direction: column; align-items: center; gap: 28px; min-width: 50%;">
                                {logo()}
                                // Mode selector — two big tiles
                                <div style="display: flex; gap: 16px;">
                                    <div style={free_style.clone()} onclick={on_select_free_roam}>
                                        <div style="font-size: 16px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase;">{"Free Roam"}</div>
                                        <div style="margin-top: 10px; font-size: 11px; font-weight: 400; letter-spacing: 1px; color: #64748B;">{"Explore + destroy"}</div>
                                    </div>
                                    <div style={cta_style.clone()} onclick={on_select_cta}>
                                        <div style="font-size: 16px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase;">{"Capture the Area"}</div>
                                        <div style="margin-top: 10px; font-size: 11px; font-weight: 400; letter-spacing: 1px; color: #64748B;">{"5v5 timed match"}</div>
                                    </div>
                                </div>
                                // Difficulty selector — wargame style
                                <div style="display: flex; gap: 10px;">
                                    <button style={cap_style} onclick={on_select_captain}>{"Captain"}</button>
                                    <button style={adm_style} onclick={on_select_admiral}>{"Admiral"}</button>
                                    <button style={fc_style} onclick={on_select_fleet_cmdr}>{"Fleet Cmdr"}</button>
                                </div>
                                // Continue button. Free Roam spawns immediately;
                                // CTA advances to the ship picker step.
                                <button
                                    id="play_button"
                                    style="
                                        display: flex; align-items: center; justify-content: center;
                                        min-width: 200px; height: 56px; padding: 0 40px;
                                        background: rgba(15,23,42,0.92);
                                        color: #4ADE80;
                                        border: 1px solid rgba(34,197,94,0.4);
                                        border-left: 3px solid #22C55E;
                                        border-radius: 2px;
                                        font-family: 'Menlo', 'SF Mono', 'Courier New', monospace;
                                        font-size: 18px; font-weight: 700;
                                        letter-spacing: 3px; text-transform: uppercase;
                                        cursor: pointer;
                                        box-shadow: 0 2px 8px rgba(0,0,0,0.5);
                                    "
                                    onclick={on_continue}
                                >
                                    {"Continue >"}
                                </button>
                            </div>
                        } else {
                            // ─── STEP 2: Ship picker (both modes) ────────────
                            <ShipPicker
                                selected={*selected_ship}
                                on_pick={on_ship_pick}
                                on_back={on_back_to_modes}
                                on_start={on_start_from_picker}
                            />
                        }
                    </Positioner>
                }
                <div style="position: fixed; bottom: 1rem; left: 50%; transform: translateX(-50%); display: flex; gap: 16px;">
                    <a href="/help/" style="color: #94A3B8; text-decoration: none; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 13px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase;">{"Help"}</a>
                    <a href="/ships/" style="color: #94A3B8; text-decoration: none; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 13px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase;">{"Ships"}</a>
                </div>
            }
        </>
    }
}

#[derive(Debug, Clone, Copy, PartialEq, SmolRoutable)]
pub enum Mk48Route {
    #[at("/about/")]
    About,
    #[at("/references/")]
    References,
    #[at("/help/")]
    Help,
    #[at("/ships/")]
    Ships,
    #[at("/ships/:selected")]
    ShipsSelected { selected: PathParam<EntityType> },
}

impl RoutableExt for Mk48Route {
    fn category(&self) -> Option<&'static str> {
        match self {
            Self::About | Self::Help | Self::Ships | Self::ShipsSelected { .. } => Some("help"),
            _ => None,
        }
    }

    fn label(&self, t: &Translator) -> String {
        match self {
            Self::Help => t.help_hint(),
            Self::About => t.about_hint(),
            Self::References => translate!(t, "References"),
            Self::Ships | Self::ShipsSelected { .. } => translate!(t, "Ships"),
        }
    }

    fn render<G: GameClient>(self) -> Html {
        match self {
            Self::About => html! {
                <AboutDialog/>
            },
            Self::References => html! {
                <ReferencesDialog/>
            },
            Self::Help => html! {
                <HelpDialog/>
            },
            Self::Ships => html! {
                <ShipsDialog/>
            },
            Self::ShipsSelected { selected } => html! {
                <ShipsDialog selected={selected.0}/>
            },
        }
    }

    fn tabs() -> impl Iterator<Item = Self> + 'static {
        [Self::Help, Self::Ships, Self::About].into_iter()
    }
}

/// Format the current match phase into a user-readable string.
fn format_match_clock(m: &MatchUpdate) -> String {
    use common::protocol::MatchPhase;
    match m.phase {
        MatchPhase::Waiting => "WAITING".to_string(),
        MatchPhase::Countdown => {
            let s = (m.remaining_ms + 999) / 1000;
            format!("{}", s.max(1))
        }
        MatchPhase::Playing => {
            let total_s = m.remaining_ms / 1000;
            let min = total_s / 60;
            let sec = total_s % 60;
            format!("{:02}:{:02}", min, sec)
        }
        MatchPhase::Ended { .. } => "ENDED".to_string(),
    }
}

/// Render the two base capture progress bars. Returns empty Html when
/// neither base is being contested.
const CAPTURE_DURATION_MS: f32 = 30_000.0;

fn render_capture_bars(m: &MatchUpdate) -> Html {
    let blue_base_pct = (m.blue_base_capture_ms as f32 / CAPTURE_DURATION_MS).clamp(0.0, 1.0);
    let red_base_pct = (m.red_base_capture_ms as f32 / CAPTURE_DURATION_MS).clamp(0.0, 1.0);

    if blue_base_pct == 0.0 && red_base_pct == 0.0 {
        return html! {};
    }

    html! {
        <div style="display: flex; flex-direction: column; gap: 4px; font-size: 10px; letter-spacing: 1px;">
            { render_bar("RED BASE", "Capturing", red_base_pct, m.red_base_capture_ms, "#60A5FA") }
            { render_bar("BLUE BASE", "Under attack", blue_base_pct, m.blue_base_capture_ms, "#F87171") }
        </div>
    }
}

/// Render one capture progress bar. `label` = target base name,
/// `status` = "Capturing" or "Under attack", `color` = fill color
/// (invader's color).
fn render_bar(label: &str, status: &str, pct: f32, ms: u32, color: &str) -> Html {
    if pct == 0.0 {
        return html! {};
    }
    let width_pct = format!("{:.0}%", pct * 100.0);
    let bar_fill_style = format!(
        "height: 6px; width: {}; background: {}; border-radius: 1px; transition: width 0.1s linear;",
        width_pct, color
    );
    let seconds = ms / 1000;
    html! {
        <div style="display: flex; flex-direction: column; gap: 2px;">
            <div style="display: flex; justify-content: space-between; text-transform: uppercase; color: #94A3B8;">
                <span>{format!("{} — {}", label, status)}</span>
                <span>{format!("{}s / 30s", seconds)}</span>
            </div>
            <div style="height: 6px; background: rgba(148,163,184,0.2); border-radius: 1px; overflow: hidden;">
                <div style={bar_fill_style}></div>
            </div>
        </div>
    }
}

/// State of UI inputs.
pub struct UiState {
    pub active: bool,
    pub submerge: bool,
    pub armament: Option<EntityType>,
    /// Touch rudder: -1.0 (left), 0.0 (center), 1.0 (right)
    pub touch_rudder: f32,
    /// Touch throttle: 0.0 (stop) to 1.0 (full)
    pub touch_throttle: f32,
    /// Touch fire request (consumed each tick)
    pub touch_fire: bool,
    /// Touch torpedo request (consumed each tick)
    pub touch_torpedo: bool,
    /// Locked target entity ID (turrets track this ship)
    pub locked_target: Option<EntityId>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active: true,
            submerge: false,
            armament: None,
            touch_rudder: 0.0,
            touch_throttle: 0.0,
            touch_fire: false,
            touch_torpedo: false,
            locked_target: None,
        }
    }
}

pub enum UiEvent {
    /// Sensors active.
    Active(bool),
    Armament(Option<EntityType>),
    Respawn(EntityType),
    Spawn {
        alias: PlayerAlias,
        entity_type: EntityType,
        /// Which game mode to spawn into. The game dispatcher sends a
        /// `SelectGameMode` command before `Spawn` so the server knows
        /// which mode the player opted into.
        game_mode: GameMode,
    },
    Submerge(bool),
    Upgrade(EntityType),
    Team(TeamRequest),
    /// Reset the current Capture the Area match and start a new one.
    PlayAgain,
    /// Quit the current Capture the Area match back to the title screen.
    QuitToTitle,
    /// Touch rudder input (-1.0 to 1.0)
    TouchRudder(f32),
    /// Touch throttle (0.0 to 1.0)
    TouchThrottle(f32),
    /// Touch fire button pressed (main guns)
    TouchFire,
    /// Touch torpedo button pressed
    TouchTorpedo,
    /// Touch zoom (positive = zoom in, negative = zoom out)
    TouchZoom(f32),
}

#[derive(PartialEq, Clone, Default)]
pub struct UiProps {
    pub fps: f32,
    pub score: u32,
    pub status: UiStatus,
    pub teams: HashMap<TeamId, TeamDto>,
    pub members: Box<[PlayerId]>,
    pub joiners: Box<[PlayerId]>,
    pub joins: Box<[TeamId]>,
    pub touch_screen: bool,
    /// Latest Capture the Area match state. `None` in Free Roam.
    pub match_update: Option<MatchUpdate>,
    /// Last ship the player picked on the title screen / ship menu.
    /// Used by the Capture the Area auto-respawn overlay.
    pub last_spawn_entity: Option<EntityType>,
    /// Snapshot of every visible ship for the CTA minimap. Empty in
    /// Free Roam.
    pub minimap_entries: Vec<MinimapEntry>,
    /// Per-frame list of visible ship name labels to render as HTML
    /// overlays (instead of the pixellated WebGL text layer). Each
    /// carries pre-projected screen-space pixel coordinates and the
    /// player-colored RGB. Refreshed every render.
    pub ship_labels: Vec<ShipLabel>,
}

/// A ship name label pre-projected to screen space. Rendered as an
/// absolutely-positioned HTML div so it uses the system font (Menlo)
/// at crisp anti-aliased vector resolution instead of the WebGL
/// bitmap-atlas font that pixellates at high zoom.
#[derive(PartialEq, Clone)]
pub struct ShipLabel {
    /// X pixel (logical, DPI-adjusted) from the left edge of the canvas.
    pub x: i32,
    /// Y pixel (logical, DPI-adjusted) from the top edge of the canvas.
    pub y: i32,
    pub alias: String,
    /// RGB color matching the team/player tint.
    pub color: [u8; 3],
}

/// Mutually exclusive statuses.
#[derive(Default, PartialEq, Clone)]
pub enum UiStatus {
    #[default]
    Spawning,
    Playing(UiStatusPlaying),
    Respawning(UiStatusRespawning),
}

#[derive(PartialEq, Clone)]
pub struct UiStatusPlaying {
    pub entity_type: EntityType,
    pub velocity: Velocity,
    pub direction: Angle,
    pub position: Vec2,
    pub altitude: Altitude,
    pub submerge: bool,
    /// Active sensors.
    pub active: bool,
    pub primary: Instruction,
    pub secondary: Instruction,
    pub armament: Option<EntityType>,
    pub armament_consumption: Box<[bool]>,
    pub team_proximity: HashMap<TeamId, f32>,
}

#[derive(PartialEq, Clone)]
pub struct UiStatusRespawning {
    pub death_reason: DeathReason,
}

impl Mk48Game {
    pub(crate) fn update_ui_props(&mut self, context: &mut ClientContext<Self>, status: UiStatus) {
        // Latch touch detection — once we've seen a touch, stay in
        // touch mode for the session. iPad Safari fires synthesized
        // mouse events after touches that would otherwise toggle
        // context.mouse.touch_screen back to false and make the
        // TouchControls overlay flicker off.
        if context.mouse.touch_screen {
            self.touch_ever_seen = true;
        }
        let in_game = !matches!(status, UiStatus::Spawning);

        // Build the minimap snapshot from match_update.players, NOT from
        // state.game.contacts. The contacts list is already filtered by
        // the server to the local player's visual range, so ships on
        // the far side of the arena never appear in it — which meant
        // the minimap only ever showed nearby ships, defeating the
        // whole point of having a minimap. match_update.players carries
        // position + team + is_you for EVERY team-assigned boat in the
        // match, full-map visibility guaranteed.
        let minimap_entries: Vec<MinimapEntry> = if let Some(m) =
            context.state.game.match_update.as_ref()
        {
            m.players
                .iter()
                .filter(|p| p.alive)
                .map(|p| MinimapEntry {
                    pos: p.pos,
                    team: Some(p.team),
                    is_you: p.is_you,
                })
                .collect()
        } else {
            Vec::new()
        };

        // Drain the per-frame ship label buffer. `std::mem::take`
        // leaves an empty Vec in its place so the next frame's
        // render loop starts clean.
        let ship_labels = std::mem::take(&mut self.ship_labels);

        let props = UiProps {
            fps: self.fps_counter.last_sample().unwrap_or(0.0),
            score: context.state.game.score,
            status,
            teams: context.state.game.teams.clone(),
            members: context.state.game.members.clone(),
            joiners: context.state.game.joiners.clone(),
            joins: context.state.game.joins.clone(),
            touch_screen: self.touch_ever_seen || context.mouse.touch_screen,
            match_update: context.state.game.match_update.clone(),
            last_spawn_entity: self.last_spawn_entity,
            minimap_entries,
            ship_labels,
        };

        context.set_ui_props(props, in_game);
    }
}
