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
use common::entity::EntityType;
use common::protocol::{TeamDto, TeamRequest};
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
    let on_play = gctw.send_ui_event_callback.reform(|alias| UiEvent::Spawn {
        alias,
        entity_type: EntityType::G5,
    });

    let margin = "0.5rem";
    let status = props.status.clone();

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
                    <TouchControls/>
                } else if let UiStatus::Respawning(respawning) = status {
                    <RespawnOverlay status={respawning} score={props.score}/>
                }
            } else {
                if let UiStatus::Spawning = status {
                    <Positioner id="spawn" position={Position::Center}>
                        <div style="display: flex; flex-direction: column; align-items: center; gap: 2rem; min-width: 50%;">
                            {logo()}
                            <button
                                id="play_button"
                                style="
                                    background-color: #549f57;
                                    border-radius: 1rem;
                                    border: 1px solid #61b365;
                                    color: white;
                                    cursor: pointer;
                                    font-size: 3.25rem;
                                    padding: 0.7rem 2rem;
                                    white-space: nowrap;
                                    min-width: 12rem;
                                    width: 100%;
                                "
                                onclick={on_play.reform(|_: MouseEvent| PlayerAlias::default())}
                            >{"Play"}</button>
                        </div>
                    </Positioner>
                }
                <div style="position: fixed; bottom: 1rem; left: 50%; transform: translateX(-50%); display: flex; gap: 2rem;">
                    <a href="/help/" style="color: rgba(255,255,255,0.6); text-decoration: none; font-family: system-ui; font-size: 0.9rem;">{"Help"}</a>
                    <a href="/ships/" style="color: rgba(255,255,255,0.6); text-decoration: none; font-family: system-ui; font-size: 0.9rem;">{"Ships"}</a>
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
    },
    Submerge(bool),
    Upgrade(EntityType),
    Team(TeamRequest),
    /// Touch rudder input (-1.0 to 1.0)
    TouchRudder(f32),
    /// Touch throttle (0.0 to 1.0)
    TouchThrottle(f32),
    /// Touch fire button pressed
    TouchFire,
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
        };

        context.set_ui_props(props, in_game);
    }
}
