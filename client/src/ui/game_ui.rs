// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::game::Mk48Game;
use crate::ui::about_dialog::AboutDialog;
use crate::ui::help_dialog::HelpDialog;
use crate::ui::hint::Hint;
use crate::ui::logo::logo;
use crate::ui::references_dialog::ReferencesDialog;
use crate::ui::respawn_overlay::RespawnOverlay;
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

#[styled_component(Mk48Ui)]
pub fn mk48_ui(props: &PropertiesWrapper<UiProps>) -> Html {
    let ctw = use_ctw();
    let nexus = ctw.escaping.is_escaping();
    let gctw = use_gctw::<Mk48Game>();

    // Currently-selected game mode on the title screen. Defaults to Free Roam.
    // Persists across re-renders of this component.
    let selected_mode = use_state(|| GameMode::FreeRoam);

    let on_play = {
        let mode = *selected_mode;
        gctw.send_ui_event_callback.reform(move |alias| UiEvent::Spawn {
            alias,
            entity_type: EntityType::G5,
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

    const SHOOT_HINT: &str = "First, select an available weapon. Then, click in the direction to fire. If you hold the click for too long, you won't shoot.";
    const HINTS: &[(&str, &[&str])] = &[
        ("Invitation links cannot currently be accepted by players that are already in game. They must send a join request instead.", &["/invite"]),
        ("If you are asking how you move, you click and hold to set your speed and direction (or use WASD).", &["how", "move"]),
        ("The controls are click and hold (or WASD) to move, click (or Space) to shoot.", &["how", "play"]),
        (SHOOT_HINT, &["how", "shoot"]),
        (SHOOT_HINT, &["how", "use weapons"]),
        (SHOOT_HINT, &["how", "fire"])
    ];

    html! {
        <>
            if matches!(status, UiStatus::Playing(_) | UiStatus::Respawning(_)) && !nexus {
                // Capture the Area HUD — timer + scores, top-middle.
                // Only renders when the server is sending match updates
                // (i.e., the player is in CTA mode).
                if let Some(m) = props.match_update {
                    <Positioner id="match_hud" position={Position::TopMiddle{margin: "0.5rem"}}>
                        <div style="display: flex; align-items: center; gap: 20px; padding: 10px 18px; background: rgba(15,23,42,0.92); border: 1px solid rgba(148,163,184,0.4); border-left: 3px solid #4ADE80; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 16px; font-weight: 700; letter-spacing: 2px; color: #E2E8F0; box-shadow: 0 2px 8px rgba(0,0,0,0.5);">
                            <div style="color: #60A5FA;">{format!("BLUE {}", m.blue_score)}</div>
                            <div style="color: #FCD34D;">{format_match_clock(&m)}</div>
                            <div style="color: #F87171;">{format!("{} RED", m.red_score)}</div>
                        </div>
                    </Positioner>
                }
                if let UiStatus::Playing(playing) = status {
                    <Positioner id="status" position={Position::BottomMiddle{margin: "0"}} max_width="45%">
                        <StatusOverlay
                            status={playing.clone()}
                            fps={gctw.settings_cache.fps_shown.then_some(props.fps)}
                        />
                    </Positioner>
                    <UpgradeOverlay
                        position={Position::TopMiddle{margin}}
                        status={playing.clone()}
                        score={props.score}
                    />
                    <Hint entity_type={playing.entity_type}/>
                    if props.touch_screen {
                        <TouchControls/>
                    }
                } else if let UiStatus::Respawning(respawning) = status {
                    <RespawnOverlay status={respawning} score={props.score}/>
                }
            } else {
                if let UiStatus::Spawning = status {
                    <Positioner id="spawn" position={Position::Center}>
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
                                <button
                                    style="display: flex; align-items: center; justify-content: center; min-width: 140px; height: 48px; padding: 0 28px; background: rgba(15,23,42,0.92); color: #4ADE80; border: 1px solid rgba(34,197,94,0.4); border-left: 3px solid #22C55E; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 14px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);"
                                    onclick={Callback::from(|_: MouseEvent| { common::Difficulty::set_global(common::Difficulty::Captain); })}
                                >{"Captain"}</button>
                                <button
                                    style="display: flex; align-items: center; justify-content: center; min-width: 140px; height: 48px; padding: 0 28px; background: rgba(15,23,42,0.92); color: #FCD34D; border: 1px solid rgba(234,179,8,0.3); border-left: 3px solid #EAB308; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 14px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);"
                                    onclick={Callback::from(|_: MouseEvent| { common::Difficulty::set_global(common::Difficulty::Admiral); })}
                                >{"Admiral"}</button>
                                <button
                                    style="display: flex; align-items: center; justify-content: center; min-width: 140px; height: 48px; padding: 0 28px; background: rgba(15,23,42,0.92); color: #F87171; border: 1px solid rgba(239,68,68,0.3); border-left: 3px solid #EF4444; border-radius: 2px; font-family: 'Menlo', 'SF Mono', 'Courier New', monospace; font-size: 14px; font-weight: 700; letter-spacing: 2px; text-transform: uppercase; cursor: pointer; box-shadow: 0 2px 8px rgba(0,0,0,0.5);"
                                    onclick={Callback::from(|_: MouseEvent| { common::Difficulty::set_global(common::Difficulty::FleetCommander); })}
                                >{"Fleet Cmdr"}</button>
                            </div>
                            // Play button — wargame primary CTA style
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
                                onclick={on_play.reform(|_: MouseEvent| PlayerAlias::default())}
                            >{"Start Game"}</button>
                        </div>
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
/// Phase 1 stub — a nicer HUD with countdown animation lands in Phase 2.
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
    pub(crate) fn update_ui_props(&self, context: &mut ClientContext<Self>, status: UiStatus) {
        let in_game = !matches!(status, UiStatus::Spawning);
        let props = UiProps {
            fps: self.fps_counter.last_sample().unwrap_or(0.0),
            score: context.state.game.score,
            status,
            teams: context.state.game.teams.clone(),
            members: context.state.game.members.clone(),
            joiners: context.state.game.joiners.clone(),
            joins: context.state.game.joins.clone(),
            touch_screen: context.mouse.touch_screen,
            match_update: context.state.game.match_update,
        };

        context.set_ui_props(props, in_game);
    }
}
