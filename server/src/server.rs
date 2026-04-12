// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::bot::*;
use crate::entities::EntityIndex;
use crate::entity_extension::EntityExtension;
use crate::match_state::{ArenaLayout, BoatSnapshot, MatchEvent, MatchState, Team};
use crate::player::*;
use crate::protocol::*;
use crate::team::TeamRepo;
use crate::terrain_pool::improve_terrain_pool;
use crate::world::World;
use common::entity::EntityData;
use common::entity::EntityType;
use common::protocol::TeamDto;
use common::protocol::TeamUpdate;
use common::protocol::{
    Command, GameMode, MatchPhase, MatchUpdate, PlayerMatchStatsDto, Update,
};
use common::terrain::ChunkSet;
use common::ticks::Ticks;
use common::util::level_to_score;
use common::MK48_CONSTANTS;
use kodiak_server::log::{error, info};
use kodiak_server::rand::{thread_rng, Rng};
use kodiak_server::{
    map_ranges, ArenaContext, ArenaService, GameConstants, Player, PlayerAlias, PlayerId, Score,
    TeamId, TeamName,
};
use std::borrow::Cow;
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

/// A game server.
pub struct Server {
    pub world: World,
    pub counter: Ticks,
    pub player: PlayerTupleRepo,
    pub team: TeamRepo<Self>,
    /// update -> get_client_update.
    team_update: Option<(Arc<[TeamDto]>, Arc<[TeamId]>)>,
    free_points: u32,
    /// Shared Capture the Area match state. Only ticks while at least one
    /// player is in `GameMode::CaptureTheArea`. Players in Free Roam are
    /// unaffected.
    pub match_state: MatchState,
}

impl Server {
    /// Handle the "Play Again" button from the CTA results screen.
    ///
    /// Despawns every boat in the world, resets match_state (bumps
    /// match_id, clears scores + capture timers, goes back to Countdown),
    /// zeros every player's match_stats, and re-assigns teams for a
    /// fresh 5v5 match. The next `tick()` will see the Countdown phase
    /// and the match rolls on.
    fn handle_play_again(&mut self) {
        info!(
            "match {} play-again requested, resetting",
            self.match_state.match_id
        );

        // Drain every boat the world currently knows about. We walk
        // players first to clear their Status references, then the
        // world's entity sectors get cleaned up organically by
        // world.update() on the next tick via the existing physics
        // path. For a hard reset we explicitly remove team-assigned
        // players' entities.
        let dead_indices: Vec<_> = self
            .player
            .iter_borrow()
            .filter_map(|p| {
                if p.match_team.is_some() {
                    if let Status::Alive { entity_index, .. } = p.status {
                        return Some(entity_index);
                    }
                }
                None
            })
            .collect();
        for idx in dead_indices {
            // Boats require a non-Debug DeathReason — Debug panics in
            // on_world_remove for boat-kind entities. Unknown is the
            // standard "admin removal" fit.
            self.world.remove(idx, common::death_reason::DeathReason::Unknown);
        }

        // Zero out every CTA player's match_stats so the next match
        // starts from a clean slate.
        for mut p in self.player.iter_borrow_mut() {
            if p.match_team.is_some() {
                p.match_stats = PlayerMatchStats::default();
                p.status = Status::Spawning;
                p.flags = Flags::default();
                // Keep match_team and selected_loadout — the player
                // stays on their team and in their ship for the new
                // match. The tick loop re-assigns late joiners.
            }
        }

        // Reset the FSM. match_id bumps so stale client MatchUpdates
        // are discarded; phase returns to Countdown for the new intro.
        self.match_state.reset();

        info!(
            "match {} countdown starting (post play-again)",
            self.match_state.match_id
        );
    }

    /// Handle the "Quit to Title" button from the CTA results screen.
    ///
    /// Flips the player's game_mode back to FreeRoam, clears their
    /// match_team, and puts them in Spawning. If this was the last
    /// CTA participant, the tick loop will naturally stop ticking
    /// match_state on the next pass.
    fn handle_quit_to_title(&mut self, player_id: PlayerId) {
        if let Some(mut player) = self.player.borrow_player_mut(player_id) {
            info!("player {:?} quit to title", player.player_id);
            player.game_mode = GameMode::FreeRoam;
            player.match_team = None;
            player.selected_loadout = None;
            player.match_stats = PlayerMatchStats::default();
            // If they still have a boat, mark it for cleanup so mk48's
            // world.update sweeps it away next tick.
            if let Status::Alive { .. } = player.status {
                player.flags.left_game = true;
            }
            player.status = Status::Spawning;
            player.flags.left_game = false;
        }
    }

    /// One-time cleanup of pre-existing static entities when a CTA
    /// match starts. Walks the world entity pool and removes every
    /// Crate, OilPlatform, and Hq carried over from the free-roam
    /// session. With `world.suppress_statics = true` set in the same
    /// tick, new statics won't spawn — so the CTA arena stays clean
    /// for the rest of the match.
    ///
    /// Uses an iterate-and-remove-one pattern because swap_remove
    /// inside world.remove() shifts adjacent indices. O(n²) but n is
    /// bounded (≤ ~200 statics per arena) and this runs exactly once
    /// per match.
    fn clear_statics(&mut self) {
        use maybe_parallel_iterator::IntoMaybeParallelIterator;

        loop {
            let to_remove = {
                let found: Option<EntityIndex> = self
                    .world
                    .entities
                    .par_iter()
                    .into_maybe_parallel_iter()
                    .find_any(|(_, entity)| {
                        matches!(
                            entity.entity_type,
                            EntityType::Crate | EntityType::OilPlatform | EntityType::Hq
                        )
                    })
                    .map(|(idx, _)| idx);
                found
            };
            match to_remove {
                Some(idx) => self.world.remove(
                    idx,
                    common::death_reason::DeathReason::Debug("cta cleanup".to_string()),
                ),
                None => break,
            }
        }
    }

    /// Build a `BoatSnapshot` for every alive boat whose player has a
    /// Blue/Red team assignment. This is the iterator that `match_state.tick`
    /// consumes to count ships inside each base and advance capture clocks.
    ///
    /// Returns a `Vec` (not a borrow) so the caller can free the shared
    /// `&self.player` / `&self.world` borrows before mutably borrowing
    /// `&mut self.match_state` for the tick call.
    fn collect_boat_snapshots(&self) -> Vec<BoatSnapshot> {
        const SPAWN_GRACE: std::time::Duration = std::time::Duration::from_secs(3);
        let now = std::time::Instant::now();
        let world = &self.world;
        self.player
            .iter_borrow()
            .filter_map(|p| {
                let team = p.match_team?;
                match p.status {
                    Status::Alive { entity_index, .. } => {
                        let entity = &world.entities[entity_index];
                        let just_respawned = p
                            .spawn_time
                            .map(|t| now.duration_since(t) < SPAWN_GRACE)
                            .unwrap_or(false);
                        Some(BoatSnapshot {
                            pos: entity.transform.position,
                            team,
                            alive: true,
                            just_respawned,
                        })
                    }
                    _ => None,
                }
            })
            .collect()
    }

    /// Credit every player on `by_team` who has an alive boat inside the
    /// captured base with +1 `captures` and a share of CAPTURE_POINTS toward
    /// their personal_points. Called when `match_state` emits a
    /// `BaseCaptured` event.
    ///
    /// `at_team` identifies which base was captured (Blue base captured by
    /// Red, or Red base captured by Blue).
    fn award_capture_stats(&mut self, by_team: Team, at_team: Team) {
        use crate::match_state::CAPTURE_POINTS;

        let base_pos = match at_team {
            Team::Blue => ArenaLayout::DEFAULT.blue_base,
            Team::Red => ArenaLayout::DEFAULT.red_base,
        };
        let base_radius_sq =
            ArenaLayout::DEFAULT.base_radius * ArenaLayout::DEFAULT.base_radius;

        // First pass: collect PlayerIds of every contributing boat so we
        // know how many ships to split CAPTURE_POINTS across. Uses an
        // immutable borrow of self.player + self.world.
        let mut contributors: Vec<PlayerId> = Vec::new();
        {
            let world = &self.world;
            for p in self.player.iter_borrow() {
                if p.match_team != Some(by_team) {
                    continue;
                }
                if let Status::Alive { entity_index, .. } = p.status {
                    let pos = world.entities[entity_index].transform.position;
                    if pos.distance_squared(base_pos) <= base_radius_sq {
                        contributors.push(p.player_id);
                    }
                }
            }
        }

        if contributors.is_empty() {
            return;
        }
        let per_ship_points = CAPTURE_POINTS / contributors.len() as u32;

        // Second pass: mutably credit each contributor.
        for id in contributors {
            if let Some(mut p) = self.player.borrow_player_mut(id) {
                p.match_stats.captures += 1;
                p.match_stats.personal_points += per_ship_points;
            }
        }
    }

    /// Hard CTA team caps. 5 per side, 10 total including the human.
    const BLUE_MAX: u32 = 5;
    const RED_MAX: u32 = 5;

    /// Assign Blue/Red teams to every player at match start.
    ///
    /// Humans (in CTA mode) always go Blue. Bots fill the remaining
    /// Blue slots up to 5, then the Red slots up to 5. Bots beyond the
    /// cap do NOT get a team — they'll quit on their next Bot::update
    /// (see bot.rs post-processing).
    ///
    /// Each bot also gets a `selected_loadout` set based on the current
    /// `match_state.ai_fleet` and their slot within their team, so the
    /// bot trait update can override the bot's Spawn command
    /// entity_type with the fleet-appropriate ship.
    ///
    /// Also despawns every alive boat owned by a team-assigned player so
    /// that pre-match free-roam boats (which mk48 scatters across the
    /// arena) are cleared out and their owners respawn cleanly through
    /// the CTA spawn override on the next tick. Without this, Red bots
    /// that happened to be near the Blue base when the match started
    /// would simply keep their existing positions — the "red bots near
    /// my base at time zero" bug.
    fn assign_match_teams(&mut self) {
        // Humans first — they're always Blue slot 0 regardless of
        // join order.
        for mut player in self.player.iter_borrow_mut() {
            if !player.is_bot() && player.game_mode == GameMode::CaptureTheArea {
                player.match_team = Some(Team::Blue);
                player.match_slot = 0;
            }
        }

        let (mut blue, mut red) = self.count_teams();
        let fleet = self.match_state.ai_fleet.clone();

        for mut player in self.player.iter_borrow_mut() {
            if !player.is_bot() {
                continue;
            }
            if player.match_team.is_some() {
                continue;
            }
            // Honor the hard cap. Extras don't get a team and will quit
            // on their next update rather than ghosting around the arena.
            let (team, slot) = if blue < Self::BLUE_MAX && blue <= red {
                let slot = blue;
                blue += 1;
                (Team::Blue, slot)
            } else if red < Self::RED_MAX {
                let slot = red;
                red += 1;
                (Team::Red, slot)
            } else {
                // Both caps full — leave match_team = None so
                // Bot::update returns BotAction::Quit on this tick.
                continue;
            };
            player.match_team = Some(team);
            let slot_u8 = slot.min(4) as u8;
            player.match_slot = slot_u8;
            player.selected_loadout = Some(fleet.ship_for_slot(slot_u8));
        }

        // Clear pre-match boats for every team-assigned player. They'll
        // respawn via the CTA spawn override on the next tick at their
        // team base.
        self.despawn_cta_participants_boats();
    }

    /// Assign late-joining bots to a team up to the cap, and set their
    /// fleet loadout. Bots that can't be assigned (both caps full) stay
    /// team-less and will quit on their next update via the check in
    /// bot.rs.
    fn assign_late_joiners(&mut self) {
        let (mut blue, mut red) = self.count_teams();
        let fleet = self.match_state.ai_fleet.clone();
        for mut player in self.player.iter_borrow_mut() {
            // Skip players already on a team. Previously this also
            // skipped humans (!player.is_bot()), which meant a human
            // joining an in-progress match never got a team — their
            // spawn silently failed the score-gated can_spawn_as
            // check because match_team stayed None. Now we assign
            // any unassigned player, human or bot, to the next open
            // slot. Humans always go Blue when possible (slot 0 if
            // available).
            if player.match_team.is_some() {
                continue;
            }
            let (team, slot) = if blue < Self::BLUE_MAX && blue <= red {
                let s = blue;
                blue += 1;
                (Team::Blue, s)
            } else if red < Self::RED_MAX {
                let s = red;
                red += 1;
                (Team::Red, s)
            } else {
                // Caps full — don't assign, bot will quit.
                continue;
            };
            player.match_team = Some(team);
            let slot_u8 = slot.min(4) as u8;
            player.match_slot = slot_u8;
            // Only override the loadout for bots. Humans already have
            // a selected_loadout from the ship picker (set in
            // Spawn::apply after this function returns).
            if player.is_bot() {
                player.selected_loadout = Some(fleet.ship_for_slot(slot_u8));
            }
        }
    }

    /// Remove every alive boat owned by a team-assigned player and put
    /// them back in Spawning status. Used by `assign_match_teams` to
    /// clear pre-match free-roam clutter. The player's next Spawn
    /// command (from Bot::update for bots, or the next human spawn
    /// request) routes through the CTA base override.
    fn despawn_cta_participants_boats(&mut self) {
        let to_remove: Vec<EntityIndex> = self
            .player
            .iter_borrow()
            .filter_map(|p| {
                if p.match_team.is_some() {
                    if let Status::Alive { entity_index, .. } = p.status {
                        return Some(entity_index);
                    }
                }
                None
            })
            .collect();
        for idx in to_remove {
            // DeathReason::Debug is only valid for non-boat entities —
            // the boat removal path in world_mutation.rs debug_asserts
            // against it. Use Unknown for administrative removals.
            self.world.remove(idx, common::death_reason::DeathReason::Unknown);
        }
        // world.remove may leave status as Dead; force Spawning so the
        // bot AI emits a fresh Spawn on its next update.
        for mut p in self.player.iter_borrow_mut() {
            if p.match_team.is_some() {
                match p.status {
                    Status::Alive { .. } | Status::Dead { .. } => {
                        p.status = Status::Spawning;
                        p.flags.left_game = false;
                    }
                    Status::Spawning => {}
                }
            }
        }
    }

    /// Count current (Blue, Red) team sizes across all players.
    fn count_teams(&self) -> (u32, u32) {
        let mut blue = 0u32;
        let mut red = 0u32;
        for player in self.player.iter_borrow() {
            match player.match_team {
                Some(Team::Blue) => blue += 1,
                Some(Team::Red) => red += 1,
                None => {}
            }
        }
        (blue, red)
    }
}

/// Stores a player, and metadata related to it. Data stored here may only be accessed when processing,
/// this client (i.e. not when processing other entities). Bots don't use this.
#[derive(Default, Debug)]
pub struct ClientData {
    pub loaded_chunks: ChunkSet,
    pub team_initialized: bool,
}

#[derive(Default)]
pub struct PlayerExtension(pub UnsafeCell<EntityExtension>);

/// This is sound because access is limited to when the entity is in scope.
unsafe impl Send for PlayerExtension {}
unsafe impl Sync for PlayerExtension {}

impl ArenaService for Server {
    const GAME_CONSTANTS: &'static GameConstants = MK48_CONSTANTS;
    const TICK_PERIOD_SECS: f32 = Ticks::PERIOD_SECS;

    /// How long a player can remain in limbo after they lose connection.
    const LIMBO: Duration = Duration::from_secs(6);

    type Bot = Bot;
    type ClientData = ClientData;
    type GameUpdate = Update;
    type GameRequest = Command;

    /// new returns a game server with the specified parameters.
    fn new(context: &mut ArenaContext<Self>) -> Self {
        Self {
            world: World::new(World::target_radius(
                context.min_players() as f32 * EntityType::FairmileD.data().visual_area(),
            )),
            counter: Ticks::ZERO,
            player: PlayerTupleRepo::default(),
            team: TeamRepo::default(),
            team_update: None,
            free_points: if context.topology.local_arena_id.realm_id.is_temporary()
                || context.topology.local_arena_id.realm_id.is_named()
            {
                level_to_score(3)
            } else if cfg!(debug_assertions) {
                level_to_score(EntityData::MAX_BOAT_LEVEL)
            } else {
                0
            },
            match_state: MatchState::new(),
        }
    }

    fn player_joined(&mut self, player_id: PlayerId, engine_player: &mut Player<Self>) {
        if !self.player.contains(player_id) {
            self.player.insert(
                player_id,
                Arc::new(PlayerTuple::new(TempPlayer::new(
                    player_id,
                    engine_player.rank(),
                ))),
            )
        }
        let mut player = self.player.borrow_player_mut(player_id).unwrap();
        if !player.is_bot() {
            player.score = self.free_points;
        } else if cfg!(debug_assertions) {
            player.score = thread_rng().gen_range(0..=self.free_points);
        };
        player.flags.left_game = false;
    }

    fn player_command(
        &mut self,
        update: Self::GameRequest,
        player_id: PlayerId,
        engine_player: &mut Player<Self>,
    ) -> Option<Update> {
        // Intercept CTA lifecycle commands that need Server-level access
        // before the generic CommandTrait dispatch runs. These mutate
        // match_state + the player repo in ways that CommandTrait (which
        // only gets &mut World) can't express.
        match update {
            Command::PlayAgain(_) => {
                self.handle_play_again();
                return None;
            }
            Command::QuitToTitle(_) => {
                self.handle_quit_to_title(player_id);
                return None;
            }
            // Spawn requests run BEFORE the next Server::tick() cycle,
            // so the first-tick team assignment hasn't happened yet.
            // Force a CTA bootstrap here if the commanding player is in
            // CTA mode, or if the match is already active — that way
            // the Spawn::apply call below sees a player.match_team and
            // routes the spawn to the team base instead of the free-
            // roam path that scatters them all over the map.
            Command::Spawn(_) => {
                let should_run_cta = {
                    let p = self.player.borrow_player(player_id);
                    let caller_is_cta = p
                        .as_ref()
                        .map(|p| p.game_mode == GameMode::CaptureTheArea)
                        .unwrap_or(false);
                    let match_running = matches!(
                        self.match_state.phase,
                        MatchPhase::Countdown | MatchPhase::Playing
                    );
                    caller_is_cta || match_running
                };
                if should_run_cta {
                    if matches!(
                        self.match_state.phase,
                        MatchPhase::Waiting | MatchPhase::Ended { .. }
                    ) {
                        self.match_state.start_match();
                        self.assign_match_teams();
                        self.clear_statics();
                        info!(
                            "match {} started (via Spawn)",
                            self.match_state.match_id
                        );
                    } else {
                        self.assign_late_joiners();
                    }
                }
            }
            _ => {}
        }

        let player_tuple = self.player.get(player_id).unwrap();
        if let Err(e) = update.as_command().apply(
            &mut self.world,
            &player_tuple,
            &self.player,
            &mut self.team,
            engine_player.invitation_accepted().cloned(),
            engine_player.rank(),
        ) {
            info!("Command resulted in {}", e);
        }
        None
    }

    fn player_quit(&mut self, player_id: PlayerId, _player: &mut Player<Self>) {
        let mut player = self.player.borrow_player_mut(player_id).unwrap();

        // If not dead, killing entity will be sufficient.
        if player.status.is_dead() {
            player.status = Status::Spawning;
        }
        // Clear player's score.
        player.score = 0;
        // Delete all player's entities (efficiently, in the next update cycle).
        player.flags.left_game = true;
    }

    fn player_left(&mut self, player_id: PlayerId, _player: &mut Player<Self>) {
        self.player.forget(player_id, &mut self.team);
    }

    fn get_game_update(
        &self,
        player_id: PlayerId,
        player: &mut Player<Self>,
    ) -> Option<Self::GameUpdate> {
        let player_tuple = self.player.get(player_id).unwrap();
        let client = player.client_mut().unwrap();
        let client = client.data_mut()?;
        let player_team_update = self.team.player_delta(player_id, &self.player).unwrap();
        let team_update = {
            let mut ret = Vec::new();
            if !client.team_initialized {
                player_tuple.borrow_player_mut().team.forget_state();
                if let Some(initializer) = self.team.initializer() {
                    ret.push(initializer);
                }
                client.team_initialized = true;
            }
            let (members, joiners, joins) = &player_team_update;
            // TODO: We could get members on a per team basis.
            if let Some(members) = members {
                ret.push(TeamUpdate::Members(members.deref().clone().into()));
            }

            if let Some(joiners) = joiners {
                ret.push(TeamUpdate::Joiners(joiners.deref().clone().into()));
            }

            if let Some(joins) = joins {
                ret.push(TeamUpdate::Joins(joins.iter().cloned().collect()));
            }

            if let Some((added, removed)) = self.team_update.as_ref() {
                if !added.is_empty() {
                    ret.push(TeamUpdate::AddedOrUpdated(Arc::clone(added)))
                }
                if !removed.is_empty() {
                    ret.push(TeamUpdate::Removed(Arc::clone(removed)))
                }
            }
            ret
        };
        // Build a MatchUpdate for CTA players so the client sees the
        // countdown/timer/score. Free Roam players get `None` and the HUD
        // stays untouched.
        let match_update = {
            let is_cta =
                player_tuple.borrow_player().game_mode == GameMode::CaptureTheArea;
            if is_cta {
                // Build the per-player stats DTO for every CTA participant.
                // Only team-assigned players show up (humans + assigned bots),
                // so the scoreboard never lists spectators.
                let my_player_id = player_tuple.borrow_player().player_id;
                let world = &self.world;
                let mut players: Vec<PlayerMatchStatsDto> = self
                    .player
                    .iter_borrow()
                    .filter_map(|p| {
                        let team = p.match_team?;
                        // Look up the player's live world position via
                        // their entity, or fall back to the team base
                        // if they're dead/unspawned (so the minimap
                        // has a sensible placeholder).
                        let (pos, alive) = match p.status {
                            Status::Alive { entity_index, .. } => {
                                (world.entities[entity_index].transform.position, true)
                            }
                            _ => {
                                let base = match team {
                                    crate::match_state::Team::Blue => {
                                        crate::match_state::ArenaLayout::DEFAULT.blue_base
                                    }
                                    crate::match_state::Team::Red => {
                                        crate::match_state::ArenaLayout::DEFAULT.red_base
                                    }
                                };
                                (base, false)
                            }
                        };
                        Some(PlayerMatchStatsDto {
                            player_id: p.player_id,
                            alias: p.alias,
                            team,
                            ship: p.match_stats.ship_class,
                            kills: p.match_stats.kills,
                            captures: p.match_stats.captures,
                            personal_points: p.match_stats.personal_points,
                            is_you: p.player_id == my_player_id,
                            pos,
                            alive,
                        })
                    })
                    .collect();
                // Sort by personal_points desc so the client doesn't have to.
                players.sort_by(|a, b| b.personal_points.cmp(&a.personal_points));

                let m = &self.match_state;
                Some(MatchUpdate {
                    match_id: m.match_id,
                    phase: m.phase,
                    remaining_ms: m.remaining.as_millis() as u32,
                    blue_score: m.blue_score,
                    red_score: m.red_score,
                    blue_base_capture_ms: m.blue_base_capture.as_millis() as u32,
                    red_base_capture_ms: m.red_base_capture.as_millis() as u32,
                    players,
                })
            } else {
                None
            }
        };

        Some(self.world.get_player_complete(player_tuple).into_update(
            self.counter,
            team_update,
            match_update,
            &mut client.loaded_chunks,
        ))
    }

    fn is_alive(&self, player_id: PlayerId) -> bool {
        let player = self.player.borrow_player(player_id).unwrap();
        !player.flags.left_game && player.status.is_alive()
    }

    fn get_team_id(&self, player_id: PlayerId) -> Option<TeamId> {
        self.player.borrow_player(player_id).unwrap().team_id()
    }

    fn get_team_name(&self, player_id: PlayerId) -> Option<TeamName> {
        self.player
            .borrow_player(player_id)
            .unwrap()
            .team_id()
            .map(|team_id| self.team.get(team_id).unwrap().name)
    }

    fn get_team_members(&self, player_id: PlayerId) -> Option<Vec<PlayerId>> {
        self.player
            .borrow_player(player_id)
            .unwrap()
            .team_id()
            .map(|team_id| self.team.get(team_id).unwrap().members.clone().into_inner())
    }

    fn get_alias(&self, player_id: PlayerId) -> PlayerAlias {
        self.player.borrow_player(player_id).unwrap().alias
    }

    fn override_alias(&mut self, player_id: PlayerId, alias: PlayerAlias) {
        self.player.borrow_player_mut(player_id).unwrap().alias = alias;
    }

    fn get_score(&self, player_id: PlayerId) -> Score {
        let status = self.player.borrow_player(player_id).unwrap();
        if status.is_alive() {
            Score::Some(status.score)
        } else {
            Score::None
        }
    }

    /// update runs server ticks.
    fn tick(&mut self, context: &mut ArenaContext<Self>) {
        if context.topology.local_arena_id.realm_id.is_public_default() {
            improve_terrain_pool();
        }

        self.counter = self.counter.next();

        // Set the spawn-statics flag BEFORE world.update so spawn_statics
        // is a no-op this tick if any player is in CTA mode. Free-roam
        // ticks keep the normal crate/platform spawning.
        let any_cta_player = self
            .player
            .iter_borrow()
            .any(|p| p.game_mode == GameMode::CaptureTheArea);
        self.world.suppress_statics = any_cta_player;

        // Capture kill events emitted during world.update so we can award
        // team points + per-player kill stats after the borrow releases.
        // (match_state.record_kill and player.match_stats need mutable
        //  access to self, which isn't possible inside the callback.)
        let mut pending_kills: Vec<(PlayerId, PlayerId)> = Vec::new();
        self.world.update(Ticks::ONE, &mut |killer, dead| {
            context.tally_victory(killer, dead);
            pending_kills.push((killer, dead));
        });

        // ─── Capture the Area match tick ─────────────────────────────────
        //
        // Only runs when at least one player has opted into CTA mode on the
        // title screen. Free Roam players are unaffected: no match clock,
        // no team HUD, no score updates.
        // (any_cta_player was computed above for the suppress_statics flag)
        if any_cta_player {
            // Clamp the world to the fixed CTA arena so edge damage kicks
            // in beyond 1200 units and the client can draw a visible wall.
            // mk48's World::update recomputes a dynamic radius based on
            // boat visual area each tick; we stomp that value back down.
            self.world.radius = ArenaLayout::DEFAULT.arena_radius;

            // First CTA player arrived? Kick the match off and assign
            // teams to every player (human = Blue, bots split 4B/5R).
            if matches!(
                self.match_state.phase,
                MatchPhase::Waiting | MatchPhase::Ended { .. }
            ) {
                self.match_state.start_match();
                self.assign_match_teams();
                // One-time sweep: remove any static entities (crates,
                // oil platforms, HQs) carried over from the prior free-
                // roam session so the CTA arena starts clean.
                self.clear_statics();
                info!(
                    "match {} started (Capture the Area)",
                    self.match_state.match_id
                );
            } else {
                // Mid-match: any newly-joined bot without a team gets
                // slotted onto whichever side has fewer ships (Red on tie).
                self.assign_late_joiners();
            }

            // Process kills recorded during world.update: increment the
            // killer's per-player kill stat + award KILL_POINTS to their
            // team via match_state.record_kill. Only kills by team-assigned
            // players (i.e. CTA participants) count. Free-roam kills are
            // ignored here and never reach the scoreboard.
            for (killer_id, _dead_id) in pending_kills.drain(..) {
                let team_to_award = if let Some(mut killer) =
                    self.player.borrow_player_mut(killer_id)
                {
                    if let Some(team) = killer.match_team {
                        killer.match_stats.kills += 1;
                        killer.match_stats.personal_points += crate::match_state::KILL_POINTS;
                        Some(team)
                    } else {
                        None
                    }
                } else {
                    None
                };
                // Release the player borrow before touching match_state.
                if let Some(team) = team_to_award {
                    self.match_state.record_kill(team);
                }
            }

            // Walk every alive, team-assigned boat into a snapshot vec
            // before mutably borrowing `match_state` for the tick.
            let snapshots = self.collect_boat_snapshots();
            let dt = Duration::from_secs_f32(Ticks::PERIOD_SECS);
            let events = self.match_state.tick(dt, snapshots.into_iter());

            for event in events {
                match event {
                    MatchEvent::PhaseChanged(phase) => {
                        info!(
                            "match {}: phase -> {:?}",
                            self.match_state.match_id, phase
                        );
                    }
                    MatchEvent::BaseCaptured { by, at } => {
                        info!(
                            "match {}: {:?} captured {:?} base",
                            self.match_state.match_id, by, at
                        );
                        // Every friendly ship inside the captured base at
                        // the moment of capture gets +1 captures + a share
                        // of CAPTURE_POINTS toward their personal_points.
                        self.award_capture_stats(by, at);
                    }
                    MatchEvent::MatchEnded {
                        winner,
                        blue_score,
                        red_score,
                    } => {
                        info!(
                            "match {} ended: winner={:?} blue={} red={}",
                            self.match_state.match_id, winner, blue_score, red_score
                        );
                    }
                }
            }
        }

        // Needs to be called before clients receive updates, but after World::update.
        self.world.terrain.pre_update();

        if self.counter.every(Ticks::from_whole_secs(60)) {
            use std::collections::{BTreeMap, HashMap};
            use std::fs::OpenOptions;
            use std::io::{Read, Seek, Write};

            let mut count_score = HashMap::<EntityType, (usize, f32)>::new();

            for player in self.player.iter_borrow() {
                if let Status::Alive { entity_index, .. } = player.status {
                    let entity = &self.world.entities[entity_index];
                    debug_assert!(entity.is_boat());

                    let (current_count, current_score) =
                        count_score.entry(entity.entity_type).or_default();
                    *current_count += 1;

                    let level = entity.data().level;
                    let level_score = level_to_score(level);
                    let next_level_score = level_to_score(level + 1);
                    let progress = map_ranges(
                        player.score as f32,
                        level_score as f32..next_level_score as f32,
                        0.0..1.0,
                        false,
                    );
                    if progress.is_finite() {
                        *current_score += progress;
                    }
                }
            }

            tokio::task::spawn_blocking(move || {
                if let Err(e) = OpenOptions::new()
                    .create(true)
                    .read(true)
                    .write(true)
                    .open(&*"playtime.json")
                    .and_then(move |mut file| {
                        let mut buf = Vec::new();
                        file.read_to_end(&mut buf)?;
                        let mut old = if let Ok(old) =
                            serde_json::from_slice::<BTreeMap<Cow<'static, str>, (u64, f32)>>(&buf)
                        {
                            old
                        } else {
                            error!("error loading old playtime.");
                            BTreeMap::new()
                        };

                        for (entity_type, (new_count, new_score)) in count_score {
                            if new_count > 0 {
                                let string: &'static str = entity_type.into();
                                let (old_count, old_score) =
                                    old.entry(Cow::Borrowed(string)).or_default();
                                *old_count = old_count.saturating_add(new_count as u64);
                                *old_score += new_score;
                            }
                        }

                        file.set_len(0)?;
                        file.rewind()?;

                        let serialized = serde_json::to_vec(&old).unwrap_or_default();
                        file.write_all(&serialized)
                    })
                {
                    error!("error logging playtime: {:?}", e);
                }
            });
        }

        self.team_update = self.team.delta(&self.player);
    }

    fn post_update(&mut self, context: &mut ArenaContext<Self>) {
        // Needs to be after clients receive updates.
        self.world.terrain.post_update();
        self.team_update = None;
        for mut player in self.player.iter_borrow_mut() {
            if player.team.team_id() != player.team.previous_team_id {
                if player.team.previous_team_id.is_some() {
                    player.flags.left_populated_team = true;
                }
                player.team.previous_team_id = player.team.team_id();
            }
        }
        self.player.real_players_live = context.players.real_players_live;
    }

    fn entities(&self) -> usize {
        self.world.arena.count_all()
    }

    fn world_size(&self) -> f32 {
        self.world.radius
    }
}
