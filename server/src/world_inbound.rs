// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::entity::Entity;
use crate::match_state::ArenaLayout;
use crate::player::PlayerTuple;
use crate::player::PlayerTupleRepo;
use crate::player::Status;
use crate::protocol::*;
use crate::server::Server;
use crate::team::TeamRepo;
use crate::world::World;
use common::angle::Angle;
use common::entity::*;
use common::protocol::*;
use common::terrain::TerrainMutation;
use common::ticks::Ticks;
use common::util::{level_to_score, score_to_level};
use common::world::{clamp_y_to_strict_area_border, outside_strict_area, ARCTIC};
use kodiak_server::glam::Vec2;
use kodiak_server::rand::{thread_rng, Rng};
use kodiak_server::{map_ranges, InvitationDto, PlayerAlias, RankNumber};
use maybe_parallel_iterator::IntoMaybeParallelIterator;
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

impl CommandTrait for Spawn {
    fn apply(
        &self,
        world: &mut World,
        player_tuple: &Arc<PlayerTuple>,
        _players: &PlayerTupleRepo,
        _teams: &mut TeamRepo<Server>,
        invitation_accepted: Option<InvitationDto>,
        rank: Option<RankNumber>,
    ) -> Result<(), &'static str> {
        let mut player = player_tuple.borrow_player_mut();

        if player.status.is_alive() || (player.status.is_dead() && player.flags.left_game) {
            return Err("cannot spawn while already alive or quitting");
        }

        // Un-quit.
        player.flags.left_game = false;

        // Update rank.
        player.rank = rank;

        // Set alias.
        if let Some(alias) = self.alias {
            player.alias = PlayerAlias::new_sanitized(alias.as_str());
        }

        if rank >= Some(RankNumber::Rank3) {
            player.score = player.score.max(level_to_score(2));
        }

        // Free Roam ship-picker — seed the starting score.
        //
        // The user explicitly picked this ship on the title screen. Bump
        // their score up to the ship's level floor so can_spawn_as
        // accepts it (in public-prod builds `free_points` is 0, so
        // without this bump only Level 1 ships would spawn) and so the
        // upgrade overlay shows the right next-level goal.
        //
        // Gated on `alias.is_some()` — present on title-screen Spawn,
        // absent on mid-session Respawn (client/src/game.rs:1824). That
        // keeps post-death respawn flow on its existing `respawn_score`
        // + ShipMenu semantics: if you die at Level 10 you respawn near
        // Level 8, pick a Level 8 ship, and your score is unchanged.
        //
        // `match_team.is_none()` scopes this to Free Roam — CTA already
        // has its own bypass below (match_team check on can_spawn_as).
        //
        // `.max(..)` (not `=`) preserves higher existing scores — debug
        // builds start everyone at level_to_score(MAX), and future code
        // paths that pre-seed score stay compatible.
        if player.match_team.is_none() && self.alias.is_some() {
            let floor = level_to_score(self.entity_type.data().level);
            player.score = player.score.max(floor);
        }

        // Remember the picked ship for Capture the Area auto-respawns.
        // Free roam players use this harmlessly too — it's just a memo.
        player.selected_loadout = Some(self.entity_type);
        // Record the ship class in match stats so the end-of-match table
        // can show which ship each player used.
        player.match_stats.ship_class = Some(self.entity_type);
        // Stamp the spawn time so the CTA capture logic can give
        // recently-respawned boats a grace period before they count as
        // defenders. Without this, a killed bot respawning at its own
        // base pauses the attacker's capture clock for several seconds.
        player.spawn_time = Some(std::time::Instant::now());

        drop(player);
        let player = player_tuple.borrow_player();

        // In Capture the Area the player has already picked a specific
        // ship on the title screen (or the bot was assigned one from the
        // fleet composition). Both come from a score-less context, so
        // mk48's score-gated can_spawn_as check rejects them and traps
        // the player in a respawn loop. Bypass the check when the player
        // has a match_team — CTA ships are explicitly chosen, not earned.
        if player.match_team.is_none()
            && !self.entity_type.can_spawn_as(player.score, true)
        {
            return Err("cannot spawn as given entity type");
        }

        // These initial positions may be overwritten later.
        let mut spawn_position = Vec2::ZERO;
        let mut spawn_radius = 0.8 * world.radius;

        let mut rng = thread_rng();

        if !(player.is_bot() && rng.gen()) {
            // Default to spawning near the center of the world, with more points making you spawn further north.
            let raw_spawn_y = map_ranges(
                score_to_level(player.score) as f32,
                1.5..(EntityData::MAX_BOAT_LEVEL - 1) as f32,
                -0.75 * world.radius..ARCTIC.min(0.75 * world.radius),
                true,
            );
            debug_assert!((-world.radius..=world.radius).contains(&raw_spawn_y));

            // Don't spawn in wrong area.
            let spawn_y = clamp_y_to_strict_area_border(self.entity_type, raw_spawn_y);

            if spawn_y.abs() > world.radius {
                return Err("unable to spawn this type of boat");
            }

            // Solve circle equation.
            let world_half_width_at_spawn_y = (world.radius.powi(2) - spawn_y.powi(2)).sqrt();
            debug_assert!(world_half_width_at_spawn_y <= world.radius);

            // Randomize horizontal a bit. This value will end up in the range
            // [-world_half_width_at_spawn_y / 2, world_half_width_at_spawn_y / 2].
            let spawn_x = (rng.gen::<f32>() - 0.5) * world_half_width_at_spawn_y;

            spawn_position = Vec2::new(spawn_x, spawn_y);
            spawn_radius = world.radius * (1.0 / 3.0);
        }

        debug_assert!(spawn_position.length() <= world.radius);

        /*
        if !player.player_id.is_bot() {
            debug!(
                "player spawning with {} points, with vertical bias {}, near {} r~{}",
                player.score, vertical_bias, spawn_position, spawn_radius
            );
        }
         */

        // Capture the Area override: if the player has a match_team
        // assignment (humans AND bots), force-spawn at a slot-specific
        // position around their team base. This bypasses mk48's
        // level-based vertical bias and any death exclusion zone — in
        // CTA you always respawn at your own base. We key on
        // match_team (not game_mode) so bots (whose game_mode stays
        // FreeRoam by default) still get routed correctly.
        //
        // Each team has 5 slots. Slots are arranged on a pentagon
        // inscribed in a circle of radius SLOT_RING_RADIUS around the
        // base center. This gives every teammate a distinct starting
        // point so they don't all cluster at (0, ±500) and fight
        // can_spawn's threshold-clearance check. The human player is
        // always slot 0 on Blue — a fixed north-most point at the
        // Blue base.
        const SLOT_RING_RADIUS: f32 = 130.0;
        let cta_team = player.match_team;
        let cta_team_spawn: Option<Vec2> = cta_team.map(|team| {
            let base = match team {
                crate::match_state::Team::Blue => ArenaLayout::DEFAULT.blue_base,
                crate::match_state::Team::Red => ArenaLayout::DEFAULT.red_base,
            };
            let slot = player.match_slot as f32;
            // Pentagon: 5 positions at 72° spacing, starting at the
            // top (toward the enemy base for more aggressive feel).
            let angle = std::f32::consts::FRAC_PI_2
                + slot * (std::f32::consts::TAU / 5.0);
            let offset = Vec2::new(angle.cos(), angle.sin()) * SLOT_RING_RADIUS;
            base + offset
        });

        if let Some(slot_pos) = cta_team_spawn {
            spawn_position = slot_pos;
            // Tight spawn_radius — since each slot has a distinct
            // starting position, the retry loop only needs to wiggle
            // a little to avoid direct collisions.
            spawn_radius = 30.0;
        }

        let exclusion_zone = if cta_team_spawn.is_some() {
            // CTA deaths never set a spawn-exclusion zone — you always
            // respawn at your own base, even if you were killed there.
            None
        } else {
            match &player.status {
                // Player is excluded from spawning too close to where another player sunk them, for
                // fairness reasons.
                Status::Dead {
                    reason,
                    position,
                    time,
                    ..
                } => {
                    // Don't spawn too far away from where you died.
                    spawn_position = *position;
                    spawn_radius = (0.4 * world.radius).clamp(1200.0, 3000.0).min(world.radius);

                    // Don't spawn right where you died either.
                    let exclusion_seconds =
                        if player.score > level_to_score(EntityData::MAX_BOAT_LEVEL / 2) {
                            20
                        } else {
                            10
                        };

                    if reason.is_due_to_player()
                        && time.elapsed() < Duration::from_secs(exclusion_seconds)
                    {
                        Some(*position)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };

        if player.team_id().is_some() || invitation_accepted.is_some() {
            // TODO: Inefficient to scan all entities; only need to scan all players. Unfortunately,
            // that data is not available here, currently.
            if let Some((_, team_boat)) = world
                .entities
                .par_iter()
                .into_maybe_parallel_iter()
                .find_any(|(_, entity)| {
                    let data = entity.data();
                    if data.kind != EntityKind::Boat {
                        return false;
                    }

                    if let Some(exclusion_zone) = exclusion_zone {
                        if entity.transform.position.distance_squared(exclusion_zone)
                            < 1100f32.powi(2)
                        {
                            return false;
                        }
                    }

                    let is_team_member = player.team_id().is_some()
                        && entity.borrow_player().team_id() == player.team_id();

                    let was_invited_by = invitation_accepted.is_some()
                        && entity.borrow_player().player_id
                            == invitation_accepted.as_ref().unwrap().player_id;

                    is_team_member || was_invited_by
                })
            {
                spawn_position = team_boat.transform.position;
                spawn_radius = team_boat.data().radius + 25.0;
            }
        }

        drop(player);

        // Pick a spawn heading that doesn't point the player immediately
        // into the world boundary.
        //
        //  * CTA: face the enemy base (crosses the arena), so the player
        //    is pointed toward the objective.
        //  * Free Roam: face the world origin from wherever we landed,
        //    so edge-hugging spawns don't immediately run aground.
        //
        // spawn_here_or_nearby's retry loop otherwise randomizes the
        // direction, so we pass preserve_direction=true to keep this.
        let desired_direction: Angle = if let Some(team) = cta_team {
            let enemy_base = match team {
                crate::match_state::Team::Blue => ArenaLayout::DEFAULT.red_base,
                crate::match_state::Team::Red => ArenaLayout::DEFAULT.blue_base,
            };
            Angle::from_vec(enemy_base - spawn_position)
        } else {
            // Free Roam: face inward toward origin if spawn is away from
            // (0, 0), otherwise default to 0.0 (east).
            if spawn_position.length_squared() > 1.0 {
                Angle::from_vec(-spawn_position)
            } else {
                Angle::ZERO
            }
        };

        let mut boat = Entity::new(self.entity_type, Some(Arc::clone(player_tuple)));
        boat.transform.position = spawn_position;
        boat.transform.direction = desired_direction;
        boat.guidance.direction_target = desired_direction;
        // Tight per-slot wander. Each slot already has a distinct
        // starting position ~130 units from the base center, so the
        // retry loop only needs a little room to shift around for
        // collision avoidance — but procedural terrain can drop an
        // island on top of a slot, and the retry loop needs a real
        // search area to find water. Floor bumped 150 → 400 so even
        // Fletcher-sized ships get a usable search ring. Essex still
        // uses its ship_radius * 4 + 50 formula (~578). 400 is still
        // visibly "at the base" (inside the 250-radius base circle
        // plus a margin) while avoiding the infinite respawn hang
        // when terrain blocks the pentagon slot.
        let max_distance_override = if cta_team.is_some() {
            let ship_radius = self.entity_type.data().radius;
            let cap = ship_radius * 4.0 + 50.0;
            Some(cap.max(400.0))
        } else {
            None
        };
        //#[cfg(debug_assertions)]
        //let begin = std::time::Instant::now();
        if world.spawn_here_or_nearby(
            boat,
            spawn_radius,
            exclusion_zone,
            true,
            max_distance_override,
        ) {
            /*
            #[cfg(debug_assertions)]
            println!(
                "took {:?} to spawn a {:?}",
                begin.elapsed(),
                self.entity_type
            );
             */
            Ok(())
        } else {
            Err("failed to find enough space to spawn")
        }
    }
}

impl CommandTrait for Control {
    fn apply(
        &self,
        world: &mut World,
        player_tuple: &Arc<PlayerTuple>,
        players: &PlayerTupleRepo,
        teams: &mut TeamRepo<Server>,
        invitation_accepted: Option<InvitationDto>,
        rank: Option<RankNumber>,
    ) -> Result<(), &'static str> {
        let mut player = player_tuple.borrow_player_mut();

        if player.flags.left_game {
            return Err("quit");
        }

        // Pre-borrow.
        let world_radius = world.radius;

        return if let Status::Alive {
            entity_index,
            aim_target,
            ..
        } = &mut player.status
        {
            let entity = &mut world.entities[*entity_index];

            // Movement
            if let Some(guidance) = self.guidance {
                entity.guidance = guidance;
            }
            *aim_target = if let Some(mut aim_target) = self.aim_target {
                sanitize_floats(aim_target.as_mut(), -world_radius * 2.0..world_radius * 2.0)?;
                Some(
                    (aim_target - entity.transform.position)
                        .clamp_length_max(entity.data().sensors.max_range())
                        + entity.transform.position,
                )
            } else {
                None
            };
            let extension = entity.extension_mut();
            extension.set_submerge(self.submerge);
            extension.set_active(self.active);

            drop(player);

            if let Some(fire) = &self.fire {
                fire.apply(
                    world,
                    player_tuple,
                    players,
                    teams,
                    invitation_accepted,
                    rank,
                )?;
            }

            if let Some(pay) = &self.pay {
                pay.apply(
                    world,
                    player_tuple,
                    players,
                    teams,
                    invitation_accepted,
                    rank,
                )?;
            }

            if let Some(hint) = &self.hint {
                hint.apply(
                    world,
                    player_tuple,
                    players,
                    teams,
                    invitation_accepted,
                    rank,
                )?;
            }

            Ok(())
        } else {
            Err("cannot control while not alive")
        };
    }
}

impl CommandTrait for Fire {
    fn apply(
        &self,
        world: &mut World,
        player_tuple: &Arc<PlayerTuple>,
        _players: &PlayerTupleRepo,
        _teams: &mut TeamRepo<Server>,
        _invitation_accepted: Option<InvitationDto>,
        _rank: Option<RankNumber>,
    ) -> Result<(), &'static str> {
        let player = player_tuple.borrow_player();

        if player.flags.left_game {
            return Err("quit");
        }

        return if let Status::Alive {
            entity_index,
            aim_target,
            ..
        } = player.status
        {
            // Prevents limited armaments from being invalidated since all limited armaments are destroyed on upgrade.
            if player.flags.upgraded {
                return Err("cannot fire right after upgrading");
            }

            let entity = &mut world.entities[entity_index];

            let data = entity.data();

            let index = self.armament_index as usize;
            if index >= data.armaments.len() {
                return Err("armament index out of bounds");
            }

            if entity.extension().reloads[index] != Ticks::ZERO {
                return Err("armament not yet reloaded");
            }

            let armament = &data.armaments[index];
            let armament_entity_data = armament.entity_type.data();

            // Can't fire if boat is a submerged former submarine.
            if entity.altitude.is_submerged()
                && !armament_entity_data.override_can_fire_underwater
                && (data.sub_kind != EntitySubKind::Submarine
                    || matches!(armament_entity_data.kind, EntityKind::Aircraft)
                    || matches!(
                        armament_entity_data.sub_kind,
                        EntitySubKind::Shell | EntitySubKind::Sam
                    ))
            {
                return Err("cannot fire while surfacing as a boat");
            }

            if let Some(turret_index) = armament.turret {
                let turret_angle = entity.extension().turrets[turret_index];
                let turret = &data.turrets[turret_index];

                // The aim may be outside the range but the turret must not be fired if the turret's
                // current angle is outside the range.
                if !turret.within_azimuth(turret_angle) {
                    return Err("invalid turret azimuth");
                }
            }

            let armament_transform =
                entity.transform + data.armament_transform(&entity.extension().turrets, index);

            if armament_entity_data.sub_kind == EntitySubKind::Depositor {
                if let Some(mut target) = aim_target {
                    // Can't deposit in arctic.
                    target.y = target.y.min(ARCTIC - 2.0 * common::terrain::SCALE);

                    // Clamp target is in valid range from depositor or error if too far.
                    const DEPOSITOR_RANGE: f32 = 60.0;
                    let depositor = armament_transform.position;
                    let pos =
                        clamp_to_range(depositor, target, DEPOSITOR_RANGE, DEPOSITOR_RANGE * 2.0)?;

                    world.terrain.modify(TerrainMutation::simple(pos, 60.0));
                } else {
                    return Err("cannot deposit without aim target");
                }
            } else {
                // Fire weapon.
                let player_arc = Arc::clone(player_tuple);

                drop(player);
                let mut armament_entity = Entity::new(armament.entity_type, Some(player_arc));

                armament_entity.transform = armament_transform;
                armament_entity.altitude = entity.altitude;

                let aim_angle = aim_target
                    .map(|aim| Angle::from(aim - armament_entity.transform.position))
                    .unwrap_or(entity.transform.direction);

                armament_entity.guidance.velocity_target = armament_entity_data.speed;
                armament_entity.guidance.direction_target = aim_angle;

                if armament.vertical {
                    // Vertically-launched armaments can be launched in any horizontal direction.
                    armament_entity.transform.direction = armament_entity.guidance.direction_target;
                }

                // Some weapons experience random deviation on launch
                let deviation = match armament_entity_data.sub_kind {
                    EntitySubKind::Rocket | EntitySubKind::RocketTorpedo => 0.05,
                    EntitySubKind::Shell => 0.01,
                    _ => 0.03,
                };
                armament_entity.transform.direction += thread_rng().gen::<Angle>() * deviation;

                if !world.spawn_here_or_nearby(armament_entity, 0.0, None, false, None) {
                    return Err("failed to fire from current location");
                }
            }

            let entity = &mut world.entities[entity_index];
            entity.consume_armament(index);
            entity.extension_mut().clear_spawn_protection();

            Ok(())
        } else {
            Err("cannot fire while not alive")
        };
    }
}

impl CommandTrait for Pay {
    fn apply(
        &self,
        world: &mut World,
        player_tuple: &Arc<PlayerTuple>,
        _players: &PlayerTupleRepo,
        _teams: &mut TeamRepo<Server>,
        _invitation_accepted: Option<InvitationDto>,
        _rank: Option<RankNumber>,
    ) -> Result<(), &'static str> {
        let mut player = player_tuple.borrow_player_mut();

        if player.flags.left_game {
            return Err("quit");
        }

        return if let Status::Alive {
            entity_index,
            aim_target: Some(target),
            ..
        } = player.status
        {
            let entity = &world.entities[entity_index];

            // Clamp pay to range or error if too far.
            let max_range = entity.data().radii().end;
            let cutoff_range = (max_range * 2.0).min(max_range + 60.0);
            let target =
                clamp_to_range(entity.transform.position, target, max_range, cutoff_range)?;

            let pay = EntityData::COIN_VALUE; // Value of coin.
            let withdraw = pay * 2; // Payment has 50% efficiency.

            if player.score < level_to_score(entity.data().level) + withdraw {
                return Err("insufficient funds");
            }

            let mut payment = Entity::new(
                EntityType::Coin,
                Some(Arc::clone(entity.player.as_ref().unwrap())),
            );

            payment.transform.position = target;
            payment.altitude = entity.altitude;

            // If payment successfully spawns, withdraw funds.
            if world.spawn_here_or_nearby(payment, 1.0, None, false, None) {
                player.score -= withdraw;
            }

            Ok(())
        } else {
            Err("cannot pay while not alive and aiming")
        };
    }
}

impl CommandTrait for Hint {
    fn apply(
        &self,
        _: &mut World,
        player_tuple: &Arc<PlayerTuple>,
        _players: &PlayerTupleRepo,
        _teams: &mut TeamRepo<Server>,
        _invitation_accepted: Option<InvitationDto>,
        _rank: Option<RankNumber>,
    ) -> Result<(), &'static str> {
        player_tuple.borrow_player_mut().hint = Hint {
            aspect: sanitize_float(self.aspect, 0.5..2.0)?,
        };
        Ok(())
    }
}

impl CommandTrait for Upgrade {
    fn apply(
        &self,
        world: &mut World,
        player_tuple: &Arc<PlayerTuple>,
        _players: &PlayerTupleRepo,
        _teams: &mut TeamRepo<Server>,
        _invitation_accepted: Option<InvitationDto>,
        _rank: Option<RankNumber>,
    ) -> Result<(), &'static str> {
        let mut player = player_tuple.borrow_player_mut();

        if player.flags.left_game {
            return Err("quit");
        }

        let status = &mut player.status;

        if let Status::Alive { entity_index, .. } = status {
            let entity = &mut world.entities[*entity_index];
            if !entity
                .entity_type
                .can_upgrade_to(self.entity_type, player.score, true)
            {
                return Err("cannot upgrade to provided entity type");
            }

            if outside_strict_area(self.entity_type, entity.transform.position) {
                return Err("cannot upgrade outside the correct area");
            }

            player.flags.upgraded = true;

            let below_full_potential = self.entity_type.data().level < score_to_level(player.score);

            drop(player);

            entity.change_entity_type(self.entity_type, &mut world.arena, below_full_potential);

            Ok(())
        } else {
            Err("cannot upgrade while not alive")
        }
    }
}

/// Returns an error if the float isn't finite. Otherwise, clamps it to the provided range.
fn sanitize_float(float: f32, valid: Range<f32>) -> Result<f32, &'static str> {
    if float.is_finite() {
        Ok(float.clamp(valid.start, valid.end))
    } else {
        Err("float not finite")
    }
}

/// Applies sanitize_float to each element.
fn sanitize_floats<'a, F: IntoIterator<Item = &'a mut f32>>(
    floats: F,
    valid: Range<f32>,
) -> Result<(), &'static str> {
    for float in floats {
        *float = sanitize_float(*float, valid.clone())?;
    }
    Ok(())
}

/// Clamps a center -> target vector to `range` and errors if it's length is greater than
/// `cutoff_range`.
fn clamp_to_range(
    center: Vec2,
    target: Vec2,
    range: f32,
    cutoff_range: f32,
) -> Result<Vec2, &'static str> {
    let delta = target - center;
    if delta.length_squared() > cutoff_range.powi(2) {
        Err("outside maximum range")
    } else {
        Ok(center + delta.clamp_length_max(range))
    }
}

impl CommandTrait for TeamRequest {
    fn apply(
        &self,
        _world: &mut World,
        player_tuple: &Arc<PlayerTuple>,
        players: &PlayerTupleRepo,
        teams: &mut TeamRepo<Server>,
        _invitation_accepted: Option<InvitationDto>,
        _rank: Option<RankNumber>,
    ) -> Result<(), &'static str> {
        let player_id = { player_tuple.borrow_player().player_id };
        teams.handle_team_request(player_id, self.clone(), players)?;
        Ok(())
    }
}
