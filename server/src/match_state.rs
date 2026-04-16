// SPDX-FileCopyrightText: 2024 Softbear, Inc.
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Match state machine for Blue vs Red team deathmatch mode.
//!
//! Manages the 5-minute match lifecycle, two-base capture scoring, and fleet
//! composition rotation. The tick function is pure and testable in isolation
//! via the [`BoatSnapshot`] input and [`MatchEvent`] output.

use common::entity::{EntityKind, EntitySubKind, EntityType};
// Re-export the shared protocol types under their local names so match_state.rs
// (and its tests) read the same as before the `common::protocol` move.
pub use common::protocol::{MatchPhase, MatchTeam as Team, MatchWinner as Winner};
use kodiak_server::glam::Vec2;
use kodiak_server::rand::{self, Rng};
use std::time::Duration;

// ─── Constants ────────────────────────────────────────────────────────────

pub const MATCH_DURATION: Duration = Duration::from_secs(300);
pub const COUNTDOWN_DURATION: Duration = Duration::from_secs(3);
pub const BASE_CAPTURE_DURATION: Duration = Duration::from_secs(30);
pub const CAPTURE_POINTS: u32 = 50;
pub const KILL_POINTS: u32 = 10;

// ─── Arena Layout ─────────────────────────────────────────────────────────

/// Physical layout of the match arena. Kept as a struct (rather than loose
/// consts) so future variants (tight kid-mode arena, large ranked arena) are
/// drop-in replacements without refactoring callsites.
#[derive(Clone, Copy, Debug)]
pub struct ArenaLayout {
    pub blue_base: Vec2,
    pub red_base: Vec2,
    pub base_radius: f32,
    pub arena_radius: f32,
}

impl ArenaLayout {
    pub const DEFAULT: Self = Self {
        // 2× base distance (±500 → ±1000). Gives ships 2000 m of
        // open water to traverse and turn in — the Phase 3b
        // steering-layer gate failed at the old 1000 m distance
        // because terrain density × ship turn radius packed too
        // tight at that scale. See
        // plans/cta-arena-expand-and-sparsen.md.
        blue_base: Vec2::new(0.0, 1000.0),
        red_base: Vec2::new(0.0, -1000.0),
        base_radius: 250.0,
        // 1500 → 3000 to match the expanded base distance. Formula
        // that motivated 3000: base offset 1000 + ~1000 roam room
        // north/south of bases + ~1000 buffer. Existing CTA tick
        // enforcement at server.rs:906 clamps world.radius to this
        // value every tick while any CTA player is present.
        arena_radius: 3000.0,
    };
}

// ─── Teams + Phase ────────────────────────────────────────────────────────
//
// `Team`, `Winner`, and `MatchPhase` are defined in `common::protocol` so
// they can cross the client/server boundary. They're re-exported above for
// local ergonomics.

// ─── Fleet Generation ─────────────────────────────────────────────────────

/// A procedurally-generated 5-ship fleet for a single match. Both teams
/// get a mirror of the same `Fleet` so the match starts balanced, but
/// the actual ships change from match to match.
///
/// This replaces the previous hardcoded BattleLine/DestroyerSquadron/etc
/// preset menu — the user wanted ship variety, not four fixed recipes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fleet {
    pub ships: [EntityType; 5],
}

impl Fleet {
    /// Lowest ship level eligible for the bot pool. Levels below this
    /// tend to be weak starter boats that don't hold up in a 5-minute
    /// match.
    const MIN_LEVEL: u8 = 3;
    /// Highest ship level eligible for the bot pool. Caps at 6 so matches
    /// don't become battleship-on-battleship slugfests every time.
    const MAX_LEVEL: u8 = 6;

    /// Pick 5 random ships from the level 3..=6 warship pool, excluding
    /// weird edge cases (submarines, minelayers, dredgers, pirates) that
    /// don't play well in CTA. Duplicates are allowed — a two-Bismarck
    /// fleet is a valid outcome. Both teams use this same fleet.
    pub fn random(rng: &mut impl Rng) -> Self {
        use kodiak_server::rand::seq::IteratorRandom;

        let pool: Vec<EntityType> = EntityType::iter()
            .filter(|e| {
                let data = e.data();
                if data.kind != EntityKind::Boat {
                    return false;
                }
                if data.level < Self::MIN_LEVEL || data.level > Self::MAX_LEVEL {
                    return false;
                }
                // Exclude sub-kinds that would break CTA balance.
                use EntitySubKind::*;
                !matches!(
                    data.sub_kind,
                    Submarine | Minelayer | Dredger | Pirate | Tanker | Icebreaker
                )
            })
            .collect();

        // Fall back to a single-ship pool if the filter somehow empties
        // — shouldn't happen with the mk48 ship catalogue but we'd
        // rather crash a single match than panic the whole server.
        if pool.is_empty() {
            let default = EntityType::iter()
                .find(|e| e.data().kind == EntityKind::Boat)
                .expect("at least one Boat entity type must exist");
            return Self {
                ships: [default; 5],
            };
        }

        let mut pick = || pool.iter().copied().choose(rng).unwrap();
        Self {
            ships: [pick(), pick(), pick(), pick(), pick()],
        }
    }

    /// Returns the ship entity type for a given slot (0..5) in this fleet.
    /// Slots beyond 4 wrap for robustness but in practice we only have 5
    /// bots per team.
    pub fn ship_for_slot(&self, slot: u8) -> EntityType {
        self.ships[(slot as usize) % self.ships.len()]
    }
}

// ─── Match State ──────────────────────────────────────────────────────────

/// Full match state. Flat data, no hidden state, no stored constants.
/// Phase is a pure FSM; time and scores live at the top level.
pub struct MatchState {
    /// Increments on each reset(). Clients use this to discard stale packets.
    pub match_id: u32,
    pub phase: MatchPhase,
    /// Remaining time in the current phase (countdown or playing).
    pub remaining: Duration,
    pub blue_score: u32,
    pub red_score: u32,
    /// Red's progress toward capturing Blue's base (0 to BASE_CAPTURE_DURATION).
    pub blue_base_capture: Duration,
    /// Blue's progress toward capturing Red's base.
    pub red_base_capture: Duration,
    /// The procedurally-generated fleet for this match. Both teams mirror it.
    pub ai_fleet: Fleet,
    pub layout: ArenaLayout,
}

impl MatchState {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        Self {
            match_id: 1,
            phase: MatchPhase::Waiting,
            remaining: Duration::ZERO,
            blue_score: 0,
            red_score: 0,
            blue_base_capture: Duration::ZERO,
            red_base_capture: Duration::ZERO,
            ai_fleet: Fleet::random(&mut rng),
            layout: ArenaLayout::DEFAULT,
        }
    }

    /// Signal that the player has selected their ship and is ready to play.
    /// Transitions from Waiting → Countdown.
    pub fn start_match(&mut self) -> Vec<MatchEvent> {
        if matches!(self.phase, MatchPhase::Waiting | MatchPhase::Ended { .. }) {
            self.phase = MatchPhase::Countdown;
            self.remaining = COUNTDOWN_DURATION;
            vec![MatchEvent::PhaseChanged(self.phase)]
        } else {
            vec![]
        }
    }

    /// Advances match state by `dt`. Returns events that occurred this tick.
    /// The caller is responsible for applying score effects and broadcasting.
    pub fn tick<'a>(
        &mut self,
        dt: Duration,
        boats: impl IntoIterator<Item = BoatSnapshot>,
    ) -> Vec<MatchEvent> {
        let mut events = Vec::new();

        match self.phase {
            MatchPhase::Waiting => {
                // No-op until start_match() is called
            }
            MatchPhase::Countdown => {
                self.remaining = self.remaining.saturating_sub(dt);
                if self.remaining.is_zero() {
                    self.phase = MatchPhase::Playing;
                    self.remaining = MATCH_DURATION;
                    events.push(MatchEvent::PhaseChanged(self.phase));
                }
            }
            MatchPhase::Playing => {
                self.remaining = self.remaining.saturating_sub(dt);
                self.tick_captures(dt, boats, &mut events);
                if self.remaining.is_zero() {
                    let winner = self.determine_winner();
                    self.phase = MatchPhase::Ended { winner };
                    events.push(MatchEvent::PhaseChanged(self.phase));
                    events.push(MatchEvent::MatchEnded {
                        winner,
                        blue_score: self.blue_score,
                        red_score: self.red_score,
                    });
                }
            }
            MatchPhase::Ended { .. } => {
                // Frozen until reset() is called
            }
        }

        events
    }

    /// Update base capture clocks based on ship positions. Called from tick()
    /// during MatchPhase::Playing.
    fn tick_captures(
        &mut self,
        dt: Duration,
        boats: impl IntoIterator<Item = BoatSnapshot>,
        events: &mut Vec<MatchEvent>,
    ) {
        // Count living ships in each base, grouped by team.
        //
        // Recently-respawned boats are EXCLUDED when they'd otherwise
        // count as defenders in their own base. A killed Red bot
        // respawning at the Red base shouldn't pause a Blue player's
        // capture clock — the bot's AI hasn't had time to move out yet.
        let mut blue_at_blue_base: u32 = 0;
        let mut red_at_blue_base: u32 = 0;
        let mut blue_at_red_base: u32 = 0;
        let mut red_at_red_base: u32 = 0;

        for boat in boats {
            if !boat.alive {
                continue;
            }
            let at_blue = boat.pos.distance(self.layout.blue_base) <= self.layout.base_radius;
            let at_red = boat.pos.distance(self.layout.red_base) <= self.layout.base_radius;
            match (boat.team, at_blue, at_red) {
                (Team::Blue, true, _) => {
                    // Defender in own base: skipped during spawn grace
                    // period. Attackers (enemy team in own base) are
                    // still counted below.
                    if !boat.just_respawned {
                        blue_at_blue_base += 1;
                    }
                }
                (Team::Red, true, _) => red_at_blue_base += 1,
                (Team::Blue, _, true) => blue_at_red_base += 1,
                (Team::Red, _, true) => {
                    if !boat.just_respawned {
                        red_at_red_base += 1;
                    }
                }
                _ => {}
            }
        }

        // Red capturing Blue base
        if red_at_blue_base > 0 && blue_at_blue_base == 0 {
            // Shared team clock, ticks faster with more attackers
            self.blue_base_capture += dt * red_at_blue_base;
            if self.blue_base_capture >= BASE_CAPTURE_DURATION {
                self.red_score += CAPTURE_POINTS;
                self.blue_base_capture = Duration::ZERO;
                events.push(MatchEvent::BaseCaptured {
                    by: Team::Red,
                    at: Team::Blue,
                });
            }
        } else if red_at_blue_base == 0 {
            // No attackers present (contested check covered by blue_at_blue_base > 0 above) → reset
            self.blue_base_capture = Duration::ZERO;
        }
        // else: contested (both teams present), clock paused at current value

        // Blue capturing Red base (mirror of above)
        if blue_at_red_base > 0 && red_at_red_base == 0 {
            self.red_base_capture += dt * blue_at_red_base;
            if self.red_base_capture >= BASE_CAPTURE_DURATION {
                self.blue_score += CAPTURE_POINTS;
                self.red_base_capture = Duration::ZERO;
                events.push(MatchEvent::BaseCaptured {
                    by: Team::Blue,
                    at: Team::Red,
                });
            }
        } else if blue_at_red_base == 0 {
            self.red_base_capture = Duration::ZERO;
        }
    }

    /// Add points for a ship kill. Called by the world mutation code when a
    /// boat sinks another boat.
    pub fn record_kill(&mut self, killer_team: Team) {
        match killer_team {
            Team::Blue => self.blue_score += KILL_POINTS,
            Team::Red => self.red_score += KILL_POINTS,
        }
    }

    /// Full reset for Play Again. Bumps match_id, zeros scores, picks new
    /// composition, and transitions back to Countdown. Caller is responsible
    /// for despawning existing boats and respawning bots with the new
    /// composition.
    pub fn reset(&mut self) {
        let mut rng = rand::thread_rng();
        self.match_id = self.match_id.wrapping_add(1);
        self.phase = MatchPhase::Countdown;
        self.remaining = COUNTDOWN_DURATION;
        self.blue_score = 0;
        self.red_score = 0;
        self.blue_base_capture = Duration::ZERO;
        self.red_base_capture = Duration::ZERO;
        self.ai_fleet = Fleet::random(&mut rng);
    }

    /// Tear down the current match and return to Waiting. Preserves
    /// match_id monotonicity so clients discard stale MatchUpdates.
    ///
    /// Unlike `reset()` (→ Countdown, used by Play Again), this leaves the
    /// match dormant. The next time a player enters CTA mode, the tick
    /// loop's `if phase == Waiting` branch picks it up and runs a full
    /// `start_match() + assign_match_teams() + clear_statics()` bootstrap.
    ///
    /// Called from `handle_quit_to_title`. Does NOT touch per-player state
    /// — the caller is responsible for clearing match_team and despawning
    /// boats.
    pub fn reset_to_waiting(&mut self) {
        let mut rng = rand::thread_rng();
        self.match_id = self.match_id.wrapping_add(1);
        self.phase = MatchPhase::Waiting;
        self.remaining = Duration::ZERO;
        self.blue_score = 0;
        self.red_score = 0;
        self.blue_base_capture = Duration::ZERO;
        self.red_base_capture = Duration::ZERO;
        self.ai_fleet = Fleet::random(&mut rng);
    }

    fn determine_winner(&self) -> Winner {
        match self.blue_score.cmp(&self.red_score) {
            std::cmp::Ordering::Greater => Winner::Blue,
            std::cmp::Ordering::Less => Winner::Red,
            std::cmp::Ordering::Equal => Winner::Draw,
        }
    }
}

impl Default for MatchState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tick Input / Output ──────────────────────────────────────────────────

/// Lightweight snapshot of a boat for match tick consumption. Decouples
/// match_state from the full Entity/Contact types for testability.
#[derive(Clone, Copy, Debug)]
pub struct BoatSnapshot {
    pub pos: Vec2,
    pub team: Team,
    pub alive: bool,
    /// True if this boat was spawned within the capture grace period
    /// (~3 seconds). Such boats don't count as defenders in their own
    /// team's base — otherwise killed bots respawning at home would
    /// keep pausing the attacker's capture clock forever.
    pub just_respawned: bool,
}

/// Events emitted during MatchState::tick(). The caller handles side effects
/// (logging, broadcasting, awarding per-player stats).
#[derive(Debug, PartialEq, Eq)]
pub enum MatchEvent {
    PhaseChanged(MatchPhase),
    /// A team captured the enemy base. `by` = capturing team, `at` = whose base was captured.
    BaseCaptured { by: Team, at: Team },
    MatchEnded {
        winner: Winner,
        blue_score: u32,
        red_score: u32,
    },
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn boat(pos: Vec2, team: Team) -> BoatSnapshot {
        BoatSnapshot {
            pos,
            team,
            alive: true,
            just_respawned: false,
        }
    }

    fn playing_state() -> MatchState {
        let mut s = MatchState::new();
        s.start_match();
        // Skip countdown
        s.tick(COUNTDOWN_DURATION, std::iter::empty());
        assert_eq!(s.phase, MatchPhase::Playing);
        s
    }

    #[test]
    fn new_state_starts_waiting() {
        let s = MatchState::new();
        assert_eq!(s.phase, MatchPhase::Waiting);
        assert_eq!(s.blue_score, 0);
        assert_eq!(s.red_score, 0);
    }

    #[test]
    fn start_match_transitions_to_countdown() {
        let mut s = MatchState::new();
        let events = s.start_match();
        assert_eq!(s.phase, MatchPhase::Countdown);
        assert_eq!(s.remaining, COUNTDOWN_DURATION);
        assert_eq!(events, vec![MatchEvent::PhaseChanged(MatchPhase::Countdown)]);
    }

    #[test]
    fn countdown_transitions_to_playing() {
        let mut s = MatchState::new();
        s.start_match();
        let events = s.tick(COUNTDOWN_DURATION, std::iter::empty());
        assert_eq!(s.phase, MatchPhase::Playing);
        assert_eq!(s.remaining, MATCH_DURATION);
        assert!(events.contains(&MatchEvent::PhaseChanged(MatchPhase::Playing)));
    }

    #[test]
    fn tick_decrements_timer() {
        let mut s = playing_state();
        let before = s.remaining;
        s.tick(Duration::from_secs(1), std::iter::empty());
        assert_eq!(s.remaining, before - Duration::from_secs(1));
    }

    #[test]
    fn playing_transitions_to_ended_at_zero() {
        let mut s = playing_state();
        let events = s.tick(MATCH_DURATION, std::iter::empty());
        assert!(matches!(s.phase, MatchPhase::Ended { .. }));
        assert!(events
            .iter()
            .any(|e| matches!(e, MatchEvent::MatchEnded { .. })));
    }

    #[test]
    fn draw_produces_draw_winner() {
        let mut s = playing_state();
        s.blue_score = 100;
        s.red_score = 100;
        s.tick(MATCH_DURATION, std::iter::empty());
        assert_eq!(s.phase, MatchPhase::Ended { winner: Winner::Draw });
    }

    #[test]
    fn blue_wins_when_score_higher() {
        let mut s = playing_state();
        s.blue_score = 200;
        s.red_score = 100;
        s.tick(MATCH_DURATION, std::iter::empty());
        assert_eq!(s.phase, MatchPhase::Ended { winner: Winner::Blue });
    }

    #[test]
    fn single_ship_captures_base_in_30_seconds() {
        let mut s = playing_state();
        let attacker = boat(ArenaLayout::DEFAULT.red_base, Team::Blue);

        // Tick 29 seconds — should not capture yet
        for _ in 0..29 {
            s.tick(Duration::from_secs(1), std::iter::once(attacker));
        }
        assert_eq!(s.blue_score, 0);

        // Tick 1 more second — should capture
        let events = s.tick(Duration::from_secs(1), std::iter::once(attacker));
        assert_eq!(s.blue_score, CAPTURE_POINTS);
        assert_eq!(s.red_base_capture, Duration::ZERO);
        assert!(events.iter().any(|e| matches!(
            e,
            MatchEvent::BaseCaptured {
                by: Team::Blue,
                at: Team::Red
            }
        )));
    }

    #[test]
    fn two_ships_capture_base_in_15_seconds() {
        let mut s = playing_state();
        let a = boat(ArenaLayout::DEFAULT.red_base, Team::Blue);
        let b = boat(ArenaLayout::DEFAULT.red_base, Team::Blue);

        // 15 seconds × 2 ships = 30 effective seconds
        for _ in 0..15 {
            s.tick(Duration::from_secs(1), [a, b]);
        }
        assert_eq!(s.blue_score, CAPTURE_POINTS);
    }

    #[test]
    fn contested_base_pauses_capture_clock() {
        let mut s = playing_state();
        let attacker = boat(ArenaLayout::DEFAULT.red_base, Team::Blue);
        let defender = boat(ArenaLayout::DEFAULT.red_base, Team::Red);

        // Attacker present for 10 seconds → clock at 10s
        for _ in 0..10 {
            s.tick(Duration::from_secs(1), std::iter::once(attacker));
        }
        assert_eq!(s.red_base_capture, Duration::from_secs(10));

        // Defender arrives → clock pauses at 10s
        for _ in 0..5 {
            s.tick(Duration::from_secs(1), [attacker, defender]);
        }
        assert_eq!(s.red_base_capture, Duration::from_secs(10));
        assert_eq!(s.blue_score, 0);
    }

    #[test]
    fn leaving_base_resets_capture_clock() {
        let mut s = playing_state();
        let attacker_inside = boat(ArenaLayout::DEFAULT.red_base, Team::Blue);
        let attacker_outside = boat(Vec2::ZERO, Team::Blue);

        // 20 seconds inside
        for _ in 0..20 {
            s.tick(Duration::from_secs(1), std::iter::once(attacker_inside));
        }
        assert_eq!(s.red_base_capture, Duration::from_secs(20));

        // Ship leaves → clock resets
        s.tick(Duration::from_secs(1), std::iter::once(attacker_outside));
        assert_eq!(s.red_base_capture, Duration::ZERO);
    }

    #[test]
    fn dead_ship_does_not_count() {
        let mut s = playing_state();
        let dead = BoatSnapshot {
            pos: ArenaLayout::DEFAULT.red_base,
            team: Team::Blue,
            alive: false,
            just_respawned: false,
        };
        for _ in 0..30 {
            s.tick(Duration::from_secs(1), std::iter::once(dead));
        }
        assert_eq!(s.blue_score, 0);
    }

    #[test]
    fn ship_outside_base_does_not_capture() {
        let mut s = playing_state();
        // Just outside the base radius
        let pos = ArenaLayout::DEFAULT.red_base
            + Vec2::new(ArenaLayout::DEFAULT.base_radius + 10.0, 0.0);
        let attacker = boat(pos, Team::Blue);

        for _ in 0..30 {
            s.tick(Duration::from_secs(1), std::iter::once(attacker));
        }
        assert_eq!(s.blue_score, 0);
        assert_eq!(s.red_base_capture, Duration::ZERO);
    }

    #[test]
    fn capture_resets_clock_for_next_capture() {
        let mut s = playing_state();
        let attacker = boat(ArenaLayout::DEFAULT.red_base, Team::Blue);

        // First capture
        for _ in 0..30 {
            s.tick(Duration::from_secs(1), std::iter::once(attacker));
        }
        assert_eq!(s.blue_score, CAPTURE_POINTS);

        // Second capture — same ship stays inside
        for _ in 0..30 {
            s.tick(Duration::from_secs(1), std::iter::once(attacker));
        }
        assert_eq!(s.blue_score, CAPTURE_POINTS * 2);
    }

    #[test]
    fn kill_awards_team_points() {
        let mut s = playing_state();
        s.record_kill(Team::Blue);
        assert_eq!(s.blue_score, KILL_POINTS);
        assert_eq!(s.red_score, 0);

        s.record_kill(Team::Red);
        assert_eq!(s.red_score, KILL_POINTS);
    }

    #[test]
    fn reset_increments_match_id_and_zeros_scores() {
        let mut s = playing_state();
        s.blue_score = 300;
        s.red_score = 200;
        s.red_base_capture = Duration::from_secs(15);
        let original_id = s.match_id;

        s.reset();
        assert_eq!(s.match_id, original_id + 1);
        assert_eq!(s.phase, MatchPhase::Countdown);
        assert_eq!(s.blue_score, 0);
        assert_eq!(s.red_score, 0);
        assert_eq!(s.blue_base_capture, Duration::ZERO);
        assert_eq!(s.red_base_capture, Duration::ZERO);
    }

    #[test]
    fn reset_to_waiting_bumps_match_id_and_parks_in_waiting() {
        // Simulate a full match that ended, then a quit-to-title teardown.
        // reset_to_waiting must bump match_id (so stale client packets are
        // discarded) and park the FSM at Waiting (so the next CTA entry
        // runs the full bootstrap path in server.tick).
        let mut s = playing_state();
        s.blue_score = 300;
        s.red_score = 120;
        s.tick(MATCH_DURATION, std::iter::empty());
        assert!(matches!(s.phase, MatchPhase::Ended { .. }));
        let prior_id = s.match_id;

        s.reset_to_waiting();
        assert_eq!(s.phase, MatchPhase::Waiting);
        assert_eq!(s.match_id, prior_id.wrapping_add(1));
        assert_eq!(s.remaining, Duration::ZERO);
        assert_eq!(s.blue_score, 0);
        assert_eq!(s.red_score, 0);
        assert_eq!(s.blue_base_capture, Duration::ZERO);
        assert_eq!(s.red_base_capture, Duration::ZERO);
    }

    #[test]
    fn waiting_phase_ignores_ticks() {
        let mut s = MatchState::new();
        s.tick(Duration::from_secs(10), std::iter::empty());
        assert_eq!(s.phase, MatchPhase::Waiting);
    }

    #[test]
    fn ended_phase_freezes_state() {
        let mut s = playing_state();
        s.blue_score = 100;
        s.tick(MATCH_DURATION, std::iter::empty());
        let score_before = s.blue_score;

        // Further ticks should not change anything
        s.tick(Duration::from_secs(10), std::iter::empty());
        assert_eq!(s.blue_score, score_before);
    }

    #[test]
    fn fleet_ship_for_slot_bounds() {
        let mut rng = rand::thread_rng();
        let fleet = Fleet::random(&mut rng);
        // Should not panic for any slot 0..5, and slots beyond length
        // wrap cleanly.
        for slot in 0..10u8 {
            let _ = fleet.ship_for_slot(slot);
        }
    }

    #[test]
    fn random_fleet_has_5_ships() {
        let mut rng = rand::thread_rng();
        let fleet = Fleet::random(&mut rng);
        assert_eq!(fleet.ships.len(), 5);
        // Every ship should actually be a boat of a reasonable level.
        for ship in fleet.ships.iter() {
            let data = ship.data();
            assert_eq!(data.kind, EntityKind::Boat);
            assert!(data.level >= Fleet::MIN_LEVEL);
            assert!(data.level <= Fleet::MAX_LEVEL);
        }
    }

    #[test]
    fn random_fleets_produce_variety() {
        // Not a strict property test, but across a hundred rolls we'd
        // expect at least two distinct fleets if generation is actually
        // random.
        let mut rng = rand::thread_rng();
        let first = Fleet::random(&mut rng);
        let mut distinct_seen = false;
        for _ in 0..100 {
            let next = Fleet::random(&mut rng);
            if next != first {
                distinct_seen = true;
                break;
            }
        }
        assert!(distinct_seen, "Fleet::random should produce variety");
    }
}
