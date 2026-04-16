// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::player::Status;
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
    /// Player IDs of CTA teammates (same match_team). Refilled per-tick by
    /// the outer trait. The inner update treats these as friendly so the
    /// bot doesn't try to attack its own side.
    cta_teammate_ids: Vec<PlayerId>,
    /// Where the bot wants to move toward in CTA — usually the enemy base
    /// for offense, or own base if it's being captured (defense).
    /// `None` in Free Roam. Sampled by the inner update as the
    /// fallback when `cta_flow_direction` is None (out-of-grid,
    /// blocked cell, or stuck suppression).
    cta_movement_target: Option<Vec2>,
    /// Flow-field-sampled desired heading. Replaces the direct-to-
    /// target pull when Some. None means: no field built (not in a
    /// CTA match), bot is on a blocked cell, or bot is stuck and
    /// we're letting local avoidance escape.
    cta_flow_direction: Option<Vec2>,
    /// Count of consecutive ticks with velocity < 1 m/s while in
    /// CTA attack mode. At 30 ticks (3 s at 10 Hz) we suppress the
    /// flow sample so the local terrain/teammate repel can escape
    /// an awkward corner. Resets to 0 as soon as speed recovers.
    cta_stuck_ticks: u16,
    /// Turn-budget speed throttle in [0.3, 1.0]. Computed by the
    /// outer update from the multi-cell forward trace; applied by
    /// the *next* inner update's `velocity_target`. The `pending_`
    /// prefix signals the one-tick latch — it is NOT the current
    /// tick's effective scale. See plans/non-holonomic-ship-steering.md.
    ///
    /// `pub` because Server's per-tick telemetry block reads this
    /// across the kodiak Player<G>.inner.bot() path to count
    /// throttled ticks.
    pub pending_speed_scale: f32,
}

impl Default for Bot {
    fn default() -> Self {
        let mut rng = thread_rng();
        let difficulty = common::Difficulty::get_global();

        fn random_level(rng: &mut ThreadRng) -> u8 {
            rng.gen_range(1..=EntityData::MAX_BOAT_LEVEL)
        }

        // Tune bot params based on difficulty.
        //
        // Per-difficulty tuple is (max_aggro, aim_radius, level_cap,
        // speed_mult, fire_cap). Bumped substantially across the board
        // — the previous values were so passive (Captain max_aggro
        // 0.04 = 4% fire chance) that bots felt like idle decorations.
        // CTA needs bots that actually engage.
        let (max_aggro, aim_radius, level_cap, _speed_mult, _fire_cap) = match difficulty {
            common::Difficulty::Captain => (0.30f32, 12.0f32, 6u8, 0.80f32, 0.5f32),
            common::Difficulty::Admiral => (0.50f32, 6.0f32, 9u8, 0.88f32, 1.0f32),
            common::Difficulty::FleetCommander => (0.75f32, 2.0f32, 10u8, 0.95f32, 1.0f32),
        };

        Self {
            // Was rng.gen::<f32>().powi(2) * max_aggro — squaring a
            // [0,1) random biases values toward zero, so most bots
            // ended up with near-zero aggression. Use a uniform half-
            // to-full range so every bot is at least decently
            // aggressive, with some variety for personality.
            aggression: rng.gen_range(max_aggro * 0.5..=max_aggro),
            steer_bias: rng.gen::<Angle>() * 0.1,
            aim_bias: gen_radius(&mut rng, aim_radius),
            level_ambition: random_level(&mut rng).min(random_level(&mut rng)).min(level_cap),
            spawned_at_least_once: false,
            was_submerging: false,
            cta_teammate_ids: Vec::new(),
            cta_movement_target: None,
            cta_flow_direction: None,
            cta_stuck_ticks: 0,
            pending_speed_scale: 1.0,
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

            // Spring force, weighted. `weight` lets callers scale
            // the contribution — essential for friendly boats,
            // where a 5-ship pentagon would otherwise sum 4 springs
            // into a ~5-magnitude cohesion/separation force and
            // drown out the objective pull entirely.
            let spring = |weighted_sum: &mut Vec2,
                          target_delta: Vec2,
                          desired_distance: f32,
                          weight: f32| {
                let distance = target_delta.length();
                let displacement = distance - desired_distance;
                *weighted_sum +=
                    target_delta * displacement / (displacement.powi(2) + 1.0) * weight;
            };

            // Terrain avoidance — short-range safety net only. Since
            // waypoint routing (see next_attack_waypoint_stateful)
            // handles global navigation around land, this ring is
            // back to a conservative close-range repel for momentum
            // overshoots and arena-edge avoidance. Pre-waypoint
            // tuning (LOOK_AHEAD=2.0, strong repel, forward_block
            // dampen) conflicted with waypoints: a single 540 m-out
            // probe hitting the coast would cancel half the
            // objective pull and leave bots drifting off-route.
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

                    // CTA-aware friendly check: a contact is friendly if
                    // it's our own ship OR its player is in our cta
                    // teammate set. Bots that share a match_team end up
                    // in the teammate set so they stop trying to
                    // torpedo each other.
                    let same_self = contact.player_id() == Some(player_id);
                    let on_my_team = contact
                        .player_id()
                        .map(|pid| self.cta_teammate_ids.contains(&pid))
                        .unwrap_or(false);
                    let friendly = same_self || on_my_team;

                    if contact_data.kind == EntityKind::Collectible {
                        attract(&mut movement, delta_position, distance_squared);
                    } else if !friendly && contact_data.kind == EntityKind::Boat {
                        // Enemy boat. Rams charge in headlong (no
                        // engagement spring); everyone else uses a
                        // spring at engagement range so the bot
                        // approaches when far and orbits when close
                        // — replacing the old "always repel" which
                        // made bots flee from enemies.
                        if data.sub_kind != EntitySubKind::Ram {
                            let engagement =
                                ((data.length + contact_data.length) * 1.5).max(200.0);
                            spring(&mut movement, delta_position, engagement, 1.0);
                        }
                    } else if !friendly
                        && !(contact_data.kind == EntityKind::Boat
                            && data.sub_kind == EntitySubKind::Ram)
                    {
                        // Enemy non-boat (weapons, aircraft) OR enemy
                        // boat (unless we're a ram charging in) —
                        // repel to dodge projectiles or avoid
                        // collisions. Friendly boats skip this branch
                        // entirely; their separation is handled by
                        // the weighted spring below.
                        repel(&mut movement, delta_position, distance_squared);
                    }

                    if friendly {
                        if contact_data.kind == EntityKind::Boat {
                            // Weight 0.2 so 4 teammates sum to ~1.0
                            // of separation force instead of ~5.2
                            // (which was drowning out the 2.5
                            // objective pull and leaving bots
                            // orbiting each other forever instead
                            // of attacking the enemy base).
                            spring(
                                &mut movement,
                                delta_position,
                                data.radius + contact_data.radius,
                                0.2,
                            );
                        }
                    } else if match contact_data.kind {
                        // Engage enemies aggressively. CTA teammate
                        // detection above already filters out friendlies,
                        // so anything reaching this branch is an enemy
                        // worth shooting at.
                        EntityKind::Boat => true,
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

            // Push the objective. Prefer the flow-field direction
            // (terrain-aware routing around islands and the arena
            // edge) when available, falling through to the direct-
            // to-target unit vector as a last resort (flow field
            // not built, bot on a blocked cell, or stuck-detection
            // suppression).
            //
            // Weight is intentionally lower in combat so the enemy-
            // engagement spring can still steer the ship toward
            // nearby targets for shooting. REVERT note: Phase 5
            // (commit a16daa5) tried `let weight = 6.0;` always,
            // hypothesizing that in-combat bots were parking at
            // engagement range because the flow pull was too weak.
            // Match-1 measurement falsified the theory:
            // enemy_base_reached stayed at 0 AND terrain_deaths
            // regressed 12 → 83 (bots charged terrain harder with
            // nothing to deflect them). Reverted to this
            // conditional. See plans/bot-objective-commitment.md.
            let in_combat = closest_enemy.is_some();
            let weight = if in_combat { 1.5 } else { 6.0 };
            if let Some(flow_dir) = self.cta_flow_direction {
                movement += flow_dir * weight;
            } else if let Some(target) = self.cta_movement_target {
                let to_target = target - boat.transform().position;
                let dist = to_target.length();
                if dist > 50.0 {
                    movement += (to_target / dist) * weight;
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
                    // Bumped from (0.65 / 0.80 / 0.90) — old values
                    // made bots feel sluggish. With CTA's tighter
                    // arena, faster bots actually push the objective.
                    //
                    // Multiplied by `pending_speed_scale` to apply
                    // the non-holonomic turn-budget throttle computed
                    // in the previous outer update. Defaults to 1.0
                    // for non-CTA bots and when the trace finds no
                    // required turn, so this is a no-op outside CTA
                    // or in open water.
                    velocity_target: data.speed * self.pending_speed_scale * match common::Difficulty::get_global() {
                        common::Difficulty::Captain => 0.80,
                        common::Difficulty::Admiral => 0.88,
                        common::Difficulty::FleetCommander => 0.95,
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

        // Populate the bot's CTA awareness fields BEFORE running the
        // inner update so the AI can use them for friendly checks and
        // objective targeting. These fields are recomputed every tick
        // so team changes / capture-state changes propagate instantly.
        {
            let my_match_team = player_tuple.borrow_player().match_team;

            let mut teammate_ids: Vec<PlayerId> = Vec::new();
            if let Some(my_team) = my_match_team {
                for p in server.player.iter_borrow() {
                    // Don't include self in the teammate set — the
                    // inner update has its own self-skip logic and we
                    // want to keep the set small.
                    if p.player_id != player_id && p.match_team == Some(my_team) {
                        teammate_ids.push(p.player_id);
                    }
                }
            }

            // Pick a movement target. If our base is being captured
            // (>5s of enemy capture progress), defend at own base;
            // otherwise push the enemy base. The actual steering
            // heading comes from the appropriate flow field
            // (built once per match at match start); the direct
            // `movement_target` Vec2 remains as a last-resort
            // fallback for the inner update when sample returns
            // None.
            struct TargetInputs {
                movement_target: Option<Vec2>,
                flow_direction: Option<Vec2>,
                /// Turn-budget speed throttle in [0.3, 1.0]. Consumed
                /// by the NEXT inner update's `velocity_target`. 1.0
                /// outside CTA and in open water where no throttle is
                /// needed.
                speed_scale: f32,
                stuck_ticks_delta: StuckDelta,
            }
            enum StuckDelta {
                Keep,      // not in CTA or not tracking
                Reset,     // bot is moving normally
                Increment, // bot is slow
            }

            let inputs: TargetInputs = match my_match_team {
                None => TargetInputs {
                    movement_target: None,
                    flow_direction: None,
                    speed_scale: 1.0,
                    stuck_ticks_delta: StuckDelta::Reset,
                },
                Some(team) => {
                    use crate::match_state::ArenaLayout;
                    let m = &server.match_state;
                    let (own_base, enemy_base, defense_progress_ms) = match team {
                        crate::match_state::Team::Blue => (
                            ArenaLayout::DEFAULT.blue_base,
                            ArenaLayout::DEFAULT.red_base,
                            m.blue_base_capture.as_millis(),
                        ),
                        crate::match_state::Team::Red => (
                            ArenaLayout::DEFAULT.red_base,
                            ArenaLayout::DEFAULT.blue_base,
                            m.red_base_capture.as_millis(),
                        ),
                    };
                    let defending = defense_progress_ms > 5_000;
                    let target = if defending { own_base } else { enemy_base };

                    // Pick the right flow field: attackers sample
                    // the field pointing at the enemy base;
                    // defenders sample the field pointing at their
                    // own base. If not alive (Spawning/Dead), no
                    // bot_pos to sample at — leave flow None.
                    // Alive → (position, heading, speed, length).
                    // Dead/Spawning → leave flow None and reset stuck
                    // counter so the next spawn starts fresh.
                    let alive_state: Option<(Vec2, Vec2, f32, f32)> = {
                        let p = player_tuple.borrow_player();
                        match p.status {
                            Status::Alive { entity_index, .. } => {
                                let e = &server.world.entities[entity_index];
                                let speed = e.transform.velocity.to_mps();
                                let length = e.entity_type.data().length;
                                Some((
                                    e.transform.position,
                                    e.transform.direction.to_vec(),
                                    speed,
                                    length,
                                ))
                            }
                            _ => None,
                        }
                    };

                    let field = if defending {
                        match team {
                            crate::match_state::Team::Blue => server.flow_to_blue.as_ref(),
                            crate::match_state::Team::Red => server.flow_to_red.as_ref(),
                        }
                    } else {
                        match team {
                            crate::match_state::Team::Blue => server.flow_to_red.as_ref(),
                            crate::match_state::Team::Red => server.flow_to_blue.as_ref(),
                        }
                    };

                    // Multi-cell forward trace for non-holonomic
                    // steering. See plans/non-holonomic-ship-steering.md.
                    //
                    // The flow field routes correctly, but ships have
                    // turn-rate × speed × cell-size mismatches that
                    // make cell-granular headings unexecutable at
                    // cruise speed. A single look-ahead sample sees
                    // only the endpoint and misses mid-path bends;
                    // this 4-sample trace picks up the tightest turn
                    // anywhere along the projected arc, and a speed
                    // throttle scales velocity down when that turn
                    // exceeds the ship's budget.
                    let (flow_direction, speed_scale) =
                        if let (Some(f), Some((pos, heading, speed, length))) =
                            (field, alive_state)
                        {
                            // Velocity-adaptive look-ahead TIME.
                            // Slow-turning ships (Iowa, longer length)
                            // need more time to plan ahead. Mirrors
                            // the turn-rate formula at
                            // common/src/transform.rs:70.
                            let turn_rate = 0.125 + 20.0 / length;
                            let time_for_90_deg =
                                std::f32::consts::FRAC_PI_2 / turn_rate;
                            // 0.6× the 90° turn time: Iowa ~4.8 s,
                            // Lürssen ~1.2 s. Clamped to [1.0, 5.0]:
                            // below 1.0 the trace collapses into a
                            // single cell; above 5.0 it reaches past
                            // tactically useful context.
                            let lookahead_secs =
                                (time_for_90_deg * 0.6).clamp(1.0, 5.0);

                            // 4-sample forward trace. ~40 bots × 4
                            // samples × 10 Hz = 1.6k flow lookups/sec
                            // against a flat cell array — negligible.
                            // Short ships (Lürssen, 1.2 s ≈ 16 m)
                            // over-sample within one cell; fine, the
                            // code path stays uniform across classes.
                            const TRACE_SAMPLES: usize = 4;
                            let heading_angle = Angle::from(heading);
                            let mut worst_turn_rad: f32 = 0.0;
                            let mut first_flow: Option<Vec2> = None;
                            let mut blocked_samples: u32 = 0;

                            for i in 1..=TRACE_SAMPLES {
                                let t = lookahead_secs
                                    * (i as f32)
                                    / (TRACE_SAMPLES as f32);
                                let sample_pos = pos + heading * speed * t;

                                // Blocked-cell handling: SKIP blocked
                                // samples rather than substituting a
                                // stand-in direction. A stand-in
                                // papers over exactly the tight-
                                // corridor case the trace exists to
                                // catch. Count them; majority-blocked
                                // forces floor below.
                                let Some(flow) = f.sample(sample_pos) else {
                                    blocked_samples += 1;
                                    continue;
                                };

                                // First non-blocked sample becomes
                                // the steering direction (closest to
                                // the ship). worst_turn_rad separately
                                // guards against committing to a
                                // sharper turn further out.
                                if first_flow.is_none() {
                                    first_flow = Some(flow);
                                }

                                // Angle::sub uses i16 wrapping_sub →
                                // wraps into [-PI, PI] by construction;
                                // no ±PI seam bug. See kodiak's
                                // common/src/math/angle.rs:285-290.
                                let flow_angle = Angle::from(flow);
                                let delta_rad = (flow_angle - heading_angle)
                                    .abs()
                                    .to_radians();
                                if delta_rad > worst_turn_rad {
                                    worst_turn_rad = delta_rad;
                                }
                            }

                            let majority_blocked =
                                blocked_samples * 2 > TRACE_SAMPLES as u32;

                            // Turn-budget throttle.
                            //
                            // over_budget crosses zero at 50% of
                            // budget — ship starts throttling BEFORE
                            // it runs out of turn capacity (three-mode
                            // AV pattern). Tuned so an Iowa at 60°
                            // turn (1.047 rad) vs. 0.96 rad budget
                            // throttles to 59% of max. Higher than
                            // 0.5 leaves no margin for next-tick drift.
                            //
                            // 0.7 slope: at 1.0× budget → ~0.65; at
                            // 2.0× budget → 0.3 (floor). Steeper
                            // cliff-drops on small excess; shallower
                            // lets too-fast ships drift into terrain.
                            //
                            // 0.3 floor: below ~3 m/s max_accel
                            // (common/src/transform.rs:96) caps
                            // angular response. Keep forward momentum.
                            let turn_budget_rad = turn_rate * lookahead_secs;
                            let slow_factor = if majority_blocked {
                                // Trace projected into land — trust
                                // that signal over the turn-budget
                                // math. Floor regardless of surviving
                                // sample deltas.
                                0.3
                            } else {
                                let over_budget = (worst_turn_rad
                                    / turn_budget_rad
                                    - 0.5)
                                    .max(0.0);
                                (1.0 - over_budget * 0.7).clamp(0.3, 1.0)
                            };

                            // If all 4 ahead-samples were blocked,
                            // fall back to the current-position
                            // sample so the inner update still has a
                            // flow vector to follow (the 0.3 floor
                            // keeps the ship slow enough to turn).
                            // Returns None only when BOTH ahead AND
                            // current-position samples are blocked.
                            let flow_out = first_flow.or_else(|| f.sample(pos));
                            (flow_out, slow_factor)
                        } else {
                            // Not alive, or no flow field →
                            // passthrough: no flow direction, no
                            // speed throttle.
                            (None, 1.0)
                        };

                    let stuck_ticks_delta = match alive_state {
                        None => StuckDelta::Reset,
                        Some((_, _, speed, _)) if speed < 1.0 => StuckDelta::Increment,
                        Some(_) => StuckDelta::Reset,
                    };

                    TargetInputs {
                        movement_target: Some(target),
                        flow_direction,
                        speed_scale,
                        stuck_ticks_delta,
                    }
                }
            };

            let bot_state = player.inner.bot_mut().unwrap();
            bot_state.cta_teammate_ids = teammate_ids;
            bot_state.cta_movement_target = inputs.movement_target;
            bot_state.pending_speed_scale = inputs.speed_scale;

            // Apply stuck-detection state transition.
            match inputs.stuck_ticks_delta {
                StuckDelta::Keep => {}
                StuckDelta::Reset => bot_state.cta_stuck_ticks = 0,
                StuckDelta::Increment => {
                    bot_state.cta_stuck_ticks = bot_state.cta_stuck_ticks.saturating_add(1);
                }
            }

            // If stuck for more than 30 ticks (3 s at 10 Hz),
            // suppress the flow direction so the bot's local
            // avoidance (terrain repel, teammate separation) can
            // push it out of whatever corner it's caught in.
            bot_state.cta_flow_direction = if bot_state.cta_stuck_ticks > 30 {
                None
            } else {
                inputs.flow_direction
            };

            // Rate-limited debug log.
            #[cfg(debug_assertions)]
            {
                use kodiak_server::log::info;
                use std::sync::atomic::{AtomicU32, Ordering};
                static COUNT: AtomicU32 = AtomicU32::new(0);
                if COUNT.fetch_add(1, Ordering::Relaxed) % 100 == 0 {
                    if let Some(team) = my_match_team {
                        let p = player_tuple.borrow_player();
                        let pos = match p.status {
                            Status::Alive { entity_index, .. } => {
                                let e = &server.world.entities[entity_index];
                                format!("({:.0},{:.0}) spd={:.1}", e.transform.position.x, e.transform.position.y, e.transform.velocity.to_mps())
                            }
                            _ => "dead".to_string(),
                        };
                        info!(
                            "bot {} {:?} slot={} flow={} stuck={} {}",
                            p.alias.as_str(),
                            team,
                            p.match_slot,
                            if bot_state.cta_flow_direction.is_some() { "Some" } else { "None" },
                            bot_state.cta_stuck_ticks,
                            pos,
                        );
                    }
                }
            }
        }

        let update = server.world.get_player_complete(player_tuple);
        let action = player
            .inner
            .bot_mut()
            .unwrap()
            .update(update, player_id, settings);

        let match_running = matches!(
            server.match_state.phase,
            common::protocol::MatchPhase::Countdown
                | common::protocol::MatchPhase::Playing
        );

        // During an active CTA match, bots must NEVER quit. The engine
        // replaces quit bots with fresh ones (match_team = None), whose
        // Bot::update then immediately quits if we also had a "no team
        // → quit" check — producing an infinite quit/replace cycle that
        // looked to the player like red ships spontaneously disappearing
        // every couple of ticks. Instead, rewrite any Quit action to a
        // plain Spawn command. The real team assignment happens further
        // down the pipeline in player_command's interceptor via
        // assign_late_joiners.
        let action = if matches!(action, BotAction::Quit) && match_running {
            BotAction::Some(Command::Spawn(Spawn {
                alias: Some(random_bot_name()),
                entity_type: EntityType::spawn_options(0, true)
                    .choose(&mut thread_rng())
                    .expect("ship pool must not be empty"),
            }))
        } else {
            action
        };

        // Post-process: in Capture the Area, the player's match_team is
        // set and selected_loadout holds the fleet ship this bot should
        // use. Override the bot's randomly-picked spawn ship with the
        // team loadout so both sides stay balanced.
        //
        // Bots with no team (fresh replacements joined mid-match) skip
        // the entity_type override and let their random ship ride — the
        // player_command interceptor will call assign_late_joiners and
        // populate match_team + selected_loadout BEFORE Spawn::apply,
        // so by the time the spawn actually lands, the fleet loadout
        // override in world_inbound.rs's ship_radius logic already sees
        // the correct ship.
        match action {
            BotAction::Some(Command::Spawn(mut spawn)) => {
                let p = player_tuple.borrow_player();
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
