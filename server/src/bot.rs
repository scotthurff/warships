// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::server::Server;
use common::altitude::Altitude;
use common::angle::Angle;
use common::complete::CompleteTrait;
use common::contact::ContactTrait;
use common::entity::*;
use common::guidance::Guidance;
use common::protocol::*;
use common::terrain;
use common::terrain::Terrain;
use kodiak_server::glam::Vec2;
use kodiak_server::rand::rngs::ThreadRng;
use kodiak_server::rand::seq::IteratorRandom;
use kodiak_server::rand::{thread_rng, Rng};
use kodiak_server::{
    gen_radius, random_bot_name, ArenaService, ArenaSettingsDto, BotAction, BotOptions, Player,
    PlayerId,
};

/// Bot implements a ship-controlling AI that is, in many ways, equivalent to a player.
#[derive(Debug)]
pub struct Bot {
    /// Chance of attacking, randomized to improve variety of bots.
    aggression: f32,
    /// Amount to offset steering by. This creates more interesting behavior.
    steer_bias: Angle,
    /// Amount to offset aiming by. This creates more interesting hit patterns.
    aim_bias: Vec2,
    /// Maximum level bot will try to upgrade to, randomized to improve variety of bots.
    level_ambition: u8,
    /// Whether the bot spawned at least once, and therefore is capable of rage-quitting.
    spawned_at_least_once: bool,
    /// The value of submerge previously sent.
    was_submerging: bool,
}

impl Default for Bot {
    fn default() -> Self {
        let mut rng = thread_rng();
        let difficulty = common::Difficulty::get_global();

        fn random_level(rng: &mut ThreadRng) -> u8 {
            rng.gen_range(1..=EntityData::MAX_BOAT_LEVEL)
        }

        // Tune bot params based on difficulty
        let (max_aggro, aim_radius, level_cap, speed_mult, fire_cap) = match difficulty {
            common::Difficulty::Captain => (0.04, 25.0, 5, 0.65, 0.4),
            common::Difficulty::Admiral => (0.1, 10.0, 8, 0.8, 1.0),
            common::Difficulty::FleetCommander => (0.2, 3.0, 10, 0.9, 1.0),
        };

        Self {
            aggression: rng.gen::<f32>().powi(2) * max_aggro,
            steer_bias: rng.gen::<Angle>() * 0.1,
            aim_bias: gen_radius(&mut rng, aim_radius),
            level_ambition: random_level(&mut rng).min(random_level(&mut rng)).min(level_cap),
            spawned_at_least_once: false,
            was_submerging: false,
        }
    }
}

impl Bot {
    /// Max aggression is now set per-difficulty in Default::default().
    const MAX_AGGRESSION: f32 = 0.2; // Upper bound, actual value depends on difficulty.

    /// Returns true if there is land or border at the given position.
    fn is_land_or_border(pos: Vec2, terrain: &Terrain, world_radius: f32) -> bool {
        if pos.length_squared() > world_radius.powi(2) {
            return true;
        }

        terrain.sample(pos).unwrap_or(Altitude::MIN) >= terrain::SAND_LEVEL
    }

    /// update processes a complete update and returns some command (or None to quit).
    fn update<'a, U: 'a + CompleteTrait<'a>>(
        &mut self,
        mut update: U,
        player_id: PlayerId,
        settings: &ArenaSettingsDto<<Server as ArenaService>::ArenaSettings>,
    ) -> BotAction<Command> {
        let aggression = self.aggression * settings.bot_aggression();
        let mut rng = thread_rng();

        let mut contacts = update.contacts();
        let terrain = update.terrain();

        if let Some(boat) = contacts
            .next()
            .filter(|c| c.is_boat() && c.player_id() == Some(player_id))
        {
            self.spawned_at_least_once = true;

            let boat_type: EntityType = boat.entity_type().unwrap();
            let data: &EntityData = boat_type.data();
            let health_percent = 1.0 - boat.damage().to_secs() / data.max_health().to_secs();

            // Weighted sums of direction vectors for various purposes.
            let mut movement = Vec2::ZERO;

            let attract = |weighted_sum: &mut Vec2, target_delta: Vec2, distance_squared: f32| {
                *weighted_sum += target_delta / (1.0 + distance_squared);
            };

            let repel = |weighted_sum: &mut Vec2, target_delta: Vec2, distance_squared: f32| {
                attract(weighted_sum, -target_delta, distance_squared);
            };

            let spring = |weighted_sum: &mut Vec2, target_delta: Vec2, desired_distance: f32| {
                let distance = target_delta.length();
                let displacement = distance - desired_distance;
                *weighted_sum = target_delta * displacement / (displacement.powi(2) + 1.0);
            };

            // Terrain.
            const SAMPLES: u32 = 10;
            for i in 0..SAMPLES {
                let angle =
                    Angle::from_radians(i as f32 * (2.0 * std::f32::consts::PI / SAMPLES as f32));
                let delta_position = angle.to_vec() * data.length;
                if Self::is_land_or_border(
                    boat.transform().position + delta_position,
                    terrain,
                    update.world_radius(),
                ) {
                    repel(&mut movement, delta_position, 0.5 * data.length.powi(2));
                }
            }

            let mut closest_enemy: Option<(U::Contact, f32)> = None;

            // Scan sensor contacts to help make decisions.
            for contact in contacts {
                if contact.id() == boat.id() {
                    // Skip processing self.
                    continue;
                }

                if let Some(contact_data) = contact.entity_type().map(EntityType::data) {
                    let delta_position = contact.transform().position - boat.transform().position;
                    let distance_squared = delta_position.length_squared();

                    let friendly = contact.player_id() == Some(player_id);

                    if contact_data.kind == EntityKind::Collectible {
                        attract(&mut movement, delta_position, distance_squared);
                    } else if (!friendly || contact_data.kind == EntityKind::Boat)
                        && !(!friendly
                            && contact_data.kind == EntityKind::Boat
                            && data.sub_kind == EntitySubKind::Ram)
                    {
                        repel(&mut movement, delta_position, distance_squared);
                    }

                    if friendly {
                        if contact_data.kind == EntityKind::Boat {
                            spring(
                                &mut movement,
                                delta_position,
                                data.radius + contact_data.radius,
                            );
                        }
                    } else if match contact_data.kind {
                        // Don't kill smol/peaceful boats unless they get too close.
                        EntityKind::Boat => {
                            (contact_data.level + 1 >= data.level
                                && !matches!(
                                    contact_data.sub_kind,
                                    EntitySubKind::Dredger | EntitySubKind::Icebreaker
                                ))
                                || contact.player_id().map(|id| id.is_bot()).unwrap_or(false)
                                || distance_squared < 1.5 * data.radius.powi(2)
                                || health_percent < 1.0 / 3.0
                        }
                        EntityKind::Aircraft => true,
                        EntityKind::Weapon => matches!(
                            contact_data.sub_kind,
                            EntitySubKind::Missile | EntitySubKind::Torpedo
                        ),
                        EntityKind::Obstacle => {
                            repel(
                                &mut movement,
                                delta_position,
                                (distance_squared - contact_data.radius.powi(2)).max(0.0),
                            );
                            false
                        }
                        _ => false,
                    } {
                        if let Some(existing) = &closest_enemy {
                            if distance_squared < existing.1 {
                                closest_enemy = Some((contact, distance_squared));
                            }
                        } else {
                            closest_enemy = Some((contact, distance_squared));
                        }
                    }
                }
            }

            let mut best_firing_solution = None;

            if let Some((enemy, _)) = closest_enemy {
                let reloads = boat.reloads();
                let enemy_data = enemy.data();
                for (i, armament) in data.armaments.iter().enumerate() {
                    if !reloads[i] {
                        // Not yet reloaded.
                        continue;
                    }

                    let armament_entity_data: &EntityData = armament.entity_type.data();
                    if !matches!(
                        armament_entity_data.kind,
                        EntityKind::Weapon | EntityKind::Aircraft | EntityKind::Decoy
                    ) {
                        continue;
                    }

                    let relevant = match enemy_data.kind {
                        EntityKind::Aircraft | EntityKind::Weapon => {
                            if enemy.altitude().is_airborne() {
                                matches!(armament_entity_data.sub_kind, EntitySubKind::Sam)
                            } else if enemy_data.sub_kind == EntitySubKind::Torpedo
                                && enemy_data.sensors.sonar.range > 0.0
                            {
                                armament_entity_data.kind == EntityKind::Decoy
                                    && armament_entity_data.sub_kind == EntitySubKind::Sonar
                            } else {
                                false
                            }
                        }
                        EntityKind::Boat => {
                            if enemy.data().level == 1
                                && armament_entity_data.sub_kind == EntitySubKind::Shell
                            {
                                // Don't attempt to sink level 1 boats with shells, as it is very
                                // toxic for new players to die in this way.
                                false
                            } else if enemy.altitude().is_submerged() {
                                matches!(
                                    armament_entity_data.sub_kind,
                                    EntitySubKind::Torpedo
                                        | EntitySubKind::Plane
                                        | EntitySubKind::Heli
                                        | EntitySubKind::DepthCharge
                                        | EntitySubKind::RocketTorpedo
                                )
                            } else {
                                matches!(
                                    armament_entity_data.sub_kind,
                                    EntitySubKind::Torpedo
                                        | EntitySubKind::Plane
                                        | EntitySubKind::Heli
                                        | EntitySubKind::DepthCharge
                                        | EntitySubKind::Rocket
                                        | EntitySubKind::Missile
                                        | EntitySubKind::Shell
                                )
                            }
                        }
                        _ => false,
                    };

                    if !relevant {
                        continue;
                    }

                    if let Some(turret_index) = armament.turret {
                        if !data.turrets[turret_index].within_azimuth(boat.turrets()[turret_index])
                        {
                            // Out of azimuth range; cannot fire.
                            continue;
                        }
                    }

                    let transform = *boat.transform() + data.armament_transform(boat.turrets(), i);
                    let angle = Angle::from(enemy.transform().position - transform.position);

                    let mut angle_diff = (angle - transform.direction).abs();
                    if armament.vertical
                        || matches!(
                            armament_entity_data.kind,
                            EntityKind::Aircraft | EntityKind::Decoy
                        )
                    {
                        angle_diff = Angle::ZERO;
                    }

                    if angle_diff > Angle::from_degrees(60.0) {
                        continue;
                    }

                    let firing_solution = (i as u8, enemy.transform().position, angle_diff);

                    if firing_solution.2
                        < best_firing_solution
                            .map(|s: (u8, Vec2, Angle)| s.2)
                            .unwrap_or(Angle::MAX)
                    {
                        best_firing_solution = Some(firing_solution);
                    }
                }
            }

            self.was_submerging = if data.sub_kind == EntitySubKind::Submarine {
                // More positive values mean want to surface, more negative values mean want to dive.
                let surface_bias = health_percent - aggression * (1.0 / Self::MAX_AGGRESSION);

                // Hysteresis.
                if self.was_submerging && surface_bias >= 0.1 {
                    false
                } else if !self.was_submerging && surface_bias <= -0.1 {
                    true
                } else {
                    self.was_submerging
                }
            } else {
                false
            };

            let mut ret = Command::Control(Control {
                guidance: Some(Guidance {
                    direction_target: Angle::from(movement) + self.steer_bias,
                    velocity_target: data.speed * match common::Difficulty::get_global() {
                        common::Difficulty::Captain => 0.65,
                        common::Difficulty::Admiral => 0.8,
                        common::Difficulty::FleetCommander => 0.9,
                    },
                }),
                submerge: self.was_submerging,
                aim_target: best_firing_solution.map(|solution| solution.1 + self.aim_bias),
                active: health_percent >= 0.5,
                fire: best_firing_solution
                    .filter(|_| rng.gen_bool((aggression as f64).min(match common::Difficulty::get_global() {
                        common::Difficulty::Captain => 0.4,
                        common::Difficulty::Admiral => 1.0,
                        common::Difficulty::FleetCommander => 1.0,
                    })))
                    .map(|sol| Fire {
                        armament_index: sol.0,
                    }),
                pay: None,
                hint: None,
            });

            if rng.gen_bool(aggression.min(0.25) as f64) && data.level < self.level_ambition {
                // Upgrade, if possible.
                if let Some(entity_type) = boat_type
                    .upgrade_options(update.score(), true)
                    .choose(&mut rng)
                {
                    ret = Command::Upgrade(Upgrade { entity_type });
                }
            }

            BotAction::Some(ret)
        } else if self.spawned_at_least_once && rng.gen_bool(1.0 / 3.0) {
            // Rage quit.
            BotAction::Quit
        } else {
            BotAction::Some(Command::Spawn(Spawn {
                alias: Some(random_bot_name()),
                entity_type: EntityType::spawn_options(0, true)
                    .choose(&mut rng)
                    .expect("there must be at least one entity type to spawn as"),
            }))
        }
    }
}

impl kodiak_server::Bot<Server> for Bot {
    // Hard-cap the bot population to exactly 9. With the 1 human player
    // that's 10 total actors — the 5v5 Capture the Area ceiling. For
    // Free Roam 9 bots is enough to feel populated on a kid-friendly
    // map without overwhelming the arena. mk48's default (min 30, max
    // 128) was flooding CTA with 20+ bots per team, way beyond the 5/5
    // assign_match_teams cap.
    const AUTO: BotOptions = BotOptions {
        min_bots: 9,
        max_bots: 9,
        bot_percent: 0,
    };

    fn update(
        server: &Server,
        player_id: PlayerId,
        player: &mut Player<Server>,
        settings: &ArenaSettingsDto<<Server as ArenaService>::ArenaSettings>,
    ) -> BotAction<<Server as ArenaService>::GameRequest> {
        let player_tuple = server.player.get(player_id).unwrap();
        let update = server.world.get_player_complete(player_tuple);
        let action = player
            .inner
            .bot_mut()
            .unwrap()
            .update(update, player_id, settings);

        // Check the match state so we can behave correctly in CTA:
        //  - if the match is running but this bot has no team, quit
        //    (the assign_late_joiners caps are full — we shouldn't
        //    linger as a ghost entity in the arena).
        //  - if the bot has a team, its spawn uses the fleet loadout.
        let match_running = matches!(
            server.match_state.phase,
            common::protocol::MatchPhase::Countdown
                | common::protocol::MatchPhase::Playing
        );

        // Post-process: in Capture the Area, the player's match_team is
        // set and selected_loadout holds the fleet ship this bot should
        // use. Override the bot's randomly-picked spawn ship with the
        // team loadout so both sides stay balanced.
        match action {
            BotAction::Some(Command::Spawn(mut spawn)) => {
                let p = player_tuple.borrow_player();
                if match_running && p.match_team.is_none() {
                    // Can't spawn in a running CTA match without a team —
                    // the team caps are full. Quit so the engine removes
                    // us and (hopefully) stops cycling.
                    return BotAction::Quit;
                }
                if p.match_team.is_some() {
                    if let Some(ship) = p.selected_loadout {
                        spawn.entity_type = ship;
                    }
                }
                BotAction::Some(Command::Spawn(spawn))
            }
            other => other,
        }
    }
}
