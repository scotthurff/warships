# feat: Blue vs Red Team Deathmatch (Two-Base Capture Mode)

> **User-facing name:** "Capture the Area"
> **Internal name:** Team Deathmatch / `MatchState` / `GameMode::CaptureTheArea`

## Overview

Add a second game mode alongside the existing free-roam: a timed **5-minute two-base capture match**. Each team has a home base that the enemy must hold for 30 continuous seconds to capture and score. Player is always on **Blue** with 4 AI allies vs 5 **Red** AI enemies. At the end of 5 minutes, the team with the most points wins. Per-player stats (kills, captures, points) are tracked and displayed on the results screen.

**Free-roam remains the default.** Players choose between "Free Roam" and "Capture the Area" on the title screen before spawning. No multiplayer lobby. Single-player vs bots in both modes.

### Game Mode Architecture

A new `GameMode` enum lives on the server and is selected per-client before spawn:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    FreeRoam,        // mk48 default — dynamic arena, no match clock, no teams
    CaptureTheArea,  // This plan — 5min, two bases, Blue vs Red, scored
}
```

**When in `FreeRoam`:**
- `MatchState::tick()` is skipped entirely
- Bots spawn unassigned (`match_team: None`), using existing mk48 AI
- Arena uses mk48's dynamic sizing
- No match HUD, no countdown, no results screen
- Behaves exactly as it does today

**When in `CaptureTheArea`:**
- `MatchState` runs, ticks captures, emits events
- Bots force-assigned to Blue/Red in 4+5 split
- Arena fixed at 1200 radius
- Match HUD, countdown overlay, results screen all active

---

## Design Decisions (all resolved)

### Core Rules

- **Match length:** Exactly 5 minutes (300 seconds), server-authoritative
- **Team composition:** 5v5 — 1 player + 4 AI allies (Blue) vs 5 AI enemies (Red)
- **Player ship selection:** Player picks any ship before each match (no level gating, fresh pick every match)
- **AI fleet composition:** Randomly rotated from 4 balanced presets (see AI Rotations). Both teams always get the identical loadout.
- **Difficulty:** Implicit — picking a bigger ship = easier (more HP, more firepower)
- **Respawn:** 2-second delay after death, respawn at your team's base with the same ship
- **World bounds:** Fixed arena radius 1200 units. Ships taking edge damage prevents free roaming.
- **Two bases (not one central zone):** Blue base north, Red base south. Scoring requires invading the enemy base.
- **Pre-match countdown:** 3 / 2 / 1 / FIGHT intro before gameplay starts
- **Reset:** Play Again calls a proper `reset()` method — instant new match, no loading screen

### Base Layout

```
       ┌─────────────────────────────┐
       │                             │
       │         ◉ BLUE BASE         │  ← Blue spawns here
       │          (0, +500)          │  ← Red must hold 30s to capture
       │                             │
       │                             │
       │      (arena 1200 r)         │
       │                             │
       │                             │
       │          (0, -500)          │  ← Red spawns here
       │         ◉ RED BASE          │  ← Blue must hold 30s to capture
       │                             │
       └─────────────────────────────┘
```

- **Blue base:** `(0, +500)`, radius 250
- **Red base:** `(0, -500)`, radius 250
- **Arena radius:** 1200 (edge damage beyond this)

### Scoring

| Event | Team points |
|-------|-------------|
| Sink an enemy ship | +10 |
| Hold enemy base for 30 continuous seconds (per capture) | +50 |
| Dying | 0 |

**Balance:** 1 capture = 5 kills in value. A dominant team with 3-4 captures ends at 300-500 points. Kills remain important for defense and opportunistic scoring, but captures are the decisive objective.

### 30-Second Base Capture Mechanic

**How the capture clock works:**

1. **Ship enters enemy base** → team capture clock starts at 0 (only if no clock already running)
2. **Each tick a friendly ship is inside the base** → clock ticks up at `1.0 * (ship_count)` seconds per second
   - 1 ship = 30 seconds to full capture
   - 2 ships = 15 seconds
   - 3 ships = 10 seconds
   - 5 ships (team stack) = 6 seconds
3. **Clock reaches 30 seconds** → **+50 team points awarded**, clock resets to 0, capture can begin again immediately
4. **Enemy ships arrive inside their own base** (contested) → clock **pauses**, does not reset
5. **All friendly ships leave the base OR are sunk** → clock **resets to 0**. You must commit.
6. **Ship ticks over 30** → any excess seconds are not banked, next capture starts fresh

**Visual feedback:**
- Progress ring around the invaded base, filling up in the invader's color (Blue ring fills on Red base when Blue captures)
- Ring pulses faster as progress approaches 100%
- At 100% → bright flash, score tick, ring resets to empty
- HUD shows "CAPTURING RED BASE  24s / 30s" when your team is capturing
- HUD shows "RED BASE UNDER ATTACK  12s / 30s" when enemies are capturing your base

**Edge cases resolved:**
- **Ship leaves at 25s:** Clock resets. No partial credit. Commits the attack.
- **Ship dies at 20s:** Clock resets if no other friendly ships remain in the base.
- **Enemy enters at 15s with 1 friendly still inside:** Clock PAUSES at 15s. When enemy leaves, clock resumes from 15s.
- **5 ships stack inside but enemy also has 5 defending:** Clock paused. Both teams are in the base but defenders cancel progress.
- **At match end (0:00), clock at 29s:** No score awarded. Must complete the 30s.

### Team Colors

- **Blue:** `#60A5FA` text, `#3B82F6` border/fill (wargame blue)
- **Red:** `#F87171` text, `#EF4444` border/fill (wargame red)
- **Base owner fill:** Translucent team color, always visible
- **Capture progress ring:** Invader team color, fills clockwise
- **Under-attack pulse:** White pulse overlay when capture is active

### AI Ship Rotations (pick 1 of 4 per match)

Each match randomly selects one composition. Both teams get the same loadout for perfect balance. Submarine Wolfpack deferred to v2.

| Name | Fleet | Flavor |
|------|-------|--------|
| **Battle Line** | 2 Battleships + 1 Cruiser + 2 Destroyers | Heavy slug-fest, long-range gun duels |
| **Destroyer Squadron** | 5 Destroyers | Fast, torpedo-heavy, chaotic |
| **Mixed Fleet** | 1 Battleship + 2 Cruisers + 2 Destroyers | Classic balanced, default feel |
| **Light Raiders** | 1 Cruiser + 4 Destroyers | Speed and skirmishing |

**v2 additions:** Submarine Wolfpack (requires "subs auto-surface in enemy base" rule to be fair)

### Per-Player Stats

At match end, display a sorted results table showing every ship in the match:

```
BLUE TEAM WINS 287 — 142

RANK  NAME       SHIP       KILLS  CAPTURES  POINTS
 1   You         Bismarck     7       3        170
 2   Blue Bot 1  Fletcher     4       1         90
 3   Red Bot 2   Kolkata      5       1         100
 4   Red Bot 1   Bismarck     3       0         30
 ...
```

**Tracked per player:**
- `kills: u32` — ships this player sank
- `captures: u32` — bases this player contributed to capturing (any ship present during a +50 tick counts)
- `personal_points: u32` — sum of kill bonuses (10 each) + capture bonuses (50 / ships_in_base)
- `ship_class: EntityType` — which ship they picked / were assigned
- `team: Team` — Blue or Red

---

## Technical Approach

### Server-side Changes

#### 1. Match state machine — `server/src/match_state.rs` (new file)

Clean state machine per Kieran's feedback — pure FSM, data flat, no hidden state, explicit events.

```rust
use std::time::Duration;
use common::entity::EntityType;

pub const MATCH_DURATION: Duration = Duration::from_secs(300);
pub const COUNTDOWN_DURATION: Duration = Duration::from_secs(3);
pub const BASE_CAPTURE_DURATION: Duration = Duration::from_secs(30);
pub const CAPTURE_POINTS: u32 = 50;
pub const KILL_POINTS: u32 = 10;

pub struct ArenaLayout {
    pub blue_base: Vec2,
    pub red_base: Vec2,
    pub base_radius: f32,
    pub arena_radius: f32,
}

impl ArenaLayout {
    pub const DEFAULT: Self = Self {
        blue_base: Vec2::new(0.0, 500.0),
        red_base: Vec2::new(0.0, -500.0),
        base_radius: 250.0,
        arena_radius: 1200.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    Blue,
    Red,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Winner {
    Blue,
    Red,
    Draw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchPhase {
    Waiting,
    Countdown,
    Playing,
    Ended { winner: Winner },
}

#[derive(Clone, Copy, Debug)]
pub enum FleetComposition {
    BattleLine,
    DestroyerSquadron,
    MixedFleet,
    LightRaiders,
}

impl FleetComposition {
    pub fn random(rng: &mut impl rand::Rng) -> Self {
        match rng.gen_range(0..4) {
            0 => Self::BattleLine,
            1 => Self::DestroyerSquadron,
            2 => Self::MixedFleet,
            _ => Self::LightRaiders,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::BattleLine => "Battle Line",
            Self::DestroyerSquadron => "Destroyer Squadron",
            Self::MixedFleet => "Mixed Fleet",
            Self::LightRaiders => "Light Raiders",
        }
    }
}

pub struct MatchState {
    pub match_id: u32,                   // Bumps on each reset() for protocol epoch
    pub phase: MatchPhase,
    pub remaining: Duration,             // Pure clock, not coupled to phase
    pub blue_score: u32,
    pub red_score: u32,
    pub blue_base_capture: Duration,     // Red's progress toward capturing Blue base
    pub red_base_capture: Duration,      // Blue's progress toward capturing Red base
    pub ai_composition: FleetComposition,
    pub layout: ArenaLayout,
}

/// Events emitted during a tick. Testable in isolation.
#[derive(Debug, PartialEq, Eq)]
pub enum MatchEvent {
    PhaseChanged(MatchPhase),
    BaseCaptured { by: Team, at: Team }, // Blue captured Red's base, etc.
    MatchEnded { winner: Winner, blue_score: u32, red_score: u32 },
}

impl MatchState {
    pub fn new() -> Self { /* ... */ }

    /// Returns a list of events emitted during this tick.
    pub fn tick(
        &mut self,
        dt: Duration,
        boats: impl Iterator<Item = BoatSnapshot>, // entity position + team + alive
    ) -> Vec<MatchEvent> {
        let mut events = Vec::new();

        match self.phase {
            MatchPhase::Waiting => { /* no-op until match starts */ }
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
                    events.push(MatchEvent::MatchEnded {
                        winner,
                        blue_score: self.blue_score,
                        red_score: self.red_score,
                    });
                }
            }
            MatchPhase::Ended { .. } => { /* frozen until reset() */ }
        }

        events
    }

    fn tick_captures(
        &mut self,
        dt: Duration,
        boats: impl Iterator<Item = BoatSnapshot>,
        events: &mut Vec<MatchEvent>,
    ) {
        // Count ships in each base by team
        let mut blue_at_blue_base = 0;
        let mut red_at_blue_base = 0;
        let mut blue_at_red_base = 0;
        let mut red_at_red_base = 0;
        for boat in boats {
            if !boat.alive { continue; }
            let at_blue = boat.pos.distance(self.layout.blue_base) <= self.layout.base_radius;
            let at_red = boat.pos.distance(self.layout.red_base) <= self.layout.base_radius;
            match (boat.team, at_blue, at_red) {
                (Team::Blue, true, _) => blue_at_blue_base += 1,
                (Team::Red, true, _) => red_at_blue_base += 1,
                (Team::Blue, _, true) => blue_at_red_base += 1,
                (Team::Red, _, true) => red_at_red_base += 1,
                _ => {}
            }
        }

        // Red capturing Blue base
        if red_at_blue_base > 0 && blue_at_blue_base == 0 {
            let rate = red_at_blue_base as u32;
            self.blue_base_capture += dt * rate;
            if self.blue_base_capture >= BASE_CAPTURE_DURATION {
                self.red_score += CAPTURE_POINTS;
                self.blue_base_capture = Duration::ZERO;
                events.push(MatchEvent::BaseCaptured { by: Team::Red, at: Team::Blue });
            }
        } else if red_at_blue_base == 0 {
            // Defenders cleared OR no one attacking → reset
            self.blue_base_capture = Duration::ZERO;
        }
        // If both present → paused, no change

        // Blue capturing Red base (symmetric)
        if blue_at_red_base > 0 && red_at_red_base == 0 {
            let rate = blue_at_red_base as u32;
            self.red_base_capture += dt * rate;
            if self.red_base_capture >= BASE_CAPTURE_DURATION {
                self.blue_score += CAPTURE_POINTS;
                self.red_base_capture = Duration::ZERO;
                events.push(MatchEvent::BaseCaptured { by: Team::Blue, at: Team::Red });
            }
        } else if blue_at_red_base == 0 {
            self.red_base_capture = Duration::ZERO;
        }
    }

    /// Full reset for Play Again. Instant, no server restart.
    pub fn reset(&mut self, rng: &mut impl rand::Rng) {
        self.match_id += 1;
        self.phase = MatchPhase::Countdown;
        self.remaining = COUNTDOWN_DURATION;
        self.blue_score = 0;
        self.red_score = 0;
        self.blue_base_capture = Duration::ZERO;
        self.red_base_capture = Duration::ZERO;
        self.ai_composition = FleetComposition::random(rng);
        // Caller must despawn boats and respawn bots separately
    }

    fn determine_winner(&self) -> Winner {
        match self.blue_score.cmp(&self.red_score) {
            std::cmp::Ordering::Greater => Winner::Blue,
            std::cmp::Ordering::Less => Winner::Red,
            std::cmp::Ordering::Equal => Winner::Draw,
        }
    }
}

#[derive(Clone, Copy)]
pub struct BoatSnapshot {
    pub pos: Vec2,
    pub team: Team,
    pub alive: bool,
}
```

**Hook into server tick loop** at `server/src/server.rs:247-249`:

```rust
let events = self.match_state.tick(Ticks::ONE.into(), world.boat_snapshots(&self.players));
for event in events {
    match event {
        MatchEvent::BaseCaptured { by, .. } => {
            log::info!("match {}: {:?} captured base", self.match_state.match_id, by);
            // Award per-player captures to all ships currently in the enemy base
            for boat in world.boats_in_zone(by_opposite_base) {
                self.players.get_mut(boat.player_id).captures += 1;
            }
        }
        MatchEvent::MatchEnded { winner, blue_score, red_score } => {
            log::info!("match {} ended: winner={:?} blue={} red={}",
                self.match_state.match_id, winner, blue_score, red_score);
        }
        MatchEvent::PhaseChanged(p) => log::debug!("phase: {:?}", p),
    }
}
```

#### 2. Per-player stats — `server/src/player.rs`

Add to `Player` struct:

```rust
pub struct Player {
    // ... existing fields
    pub team: Option<Team>,                    // Renamed from team_color per Kieran
    pub selected_loadout: Option<EntityType>,  // Player's picked ship (for respawn)
    pub match_stats: PlayerMatchStats,
}

#[derive(Default, Clone)]
pub struct PlayerMatchStats {
    pub kills: u32,
    pub captures: u32,
    pub personal_points: u32,
    pub ship_class: Option<EntityType>,
}
```

On `reset()`, zero `match_stats` for all players.

#### 3. Kill scoring — `server/src/world_mutation.rs:135`

Existing code awards `score` to killer. Also award team points and stat:

```rust
other_player.score += kill_score(e_score, other_player.score);
other_player.match_stats.kills += 1;
other_player.match_stats.personal_points += KILL_POINTS;
if let Some(team) = other_player.team {
    match team {
        Team::Blue => match_state.blue_score += KILL_POINTS,
        Team::Red => match_state.red_score += KILL_POINTS,
    }
}
```

#### 4. Bot spawning with rotating compositions — `server/src/bot.rs`

Replace `Bot::default()` line 366-371 with team + composition-aware spawning:

```rust
impl Bot {
    pub fn spawn_for_team(team: Team, slot: u8, composition: FleetComposition) -> EntityType {
        debug_assert!(slot < 5, "slot must be 0..5");
        use FleetComposition::*;
        use EntityType::*;
        let ships: [EntityType; 5] = match composition {
            BattleLine => [Bismarck, Bismarck, Kolkata, Fletcher, Fletcher],
            DestroyerSquadron => [Fletcher; 5],
            MixedFleet => [Bismarck, Kolkata, Kolkata, Fletcher, Fletcher],
            LightRaiders => [Kolkata, Fletcher, Fletcher, Fletcher, Fletcher],
        };
        // team param reserved for future cosmetic variants (paint schemes, flags)
        let _ = team;
        ships[slot as usize]
    }
}
```

#### 5. Fixed arena radius — `server/src/world.rs:89-93`

```rust
fn target_radius(&self) -> f32 {
    ArenaLayout::DEFAULT.arena_radius
}
```

#### 6. Respawn with stored loadout — `server/src/world_inbound.rs`

On death, after 2-second delay, respawn at team base with `selected_loadout`:

```rust
if player.status.is_dead() && player.death_time.elapsed() >= Duration::from_secs(2) {
    let spawn_pos = match player.team {
        Some(Team::Blue) => ArenaLayout::DEFAULT.blue_base,
        Some(Team::Red) => ArenaLayout::DEFAULT.red_base,
        None => Vec2::ZERO,
    };
    let ship = player.selected_loadout
        .or_else(|| bot_ship_from_slot(player))
        .unwrap_or(EntityType::G5);
    world.spawn_at(spawn_pos, ship, player.player_id);
}
```

#### 7. Protocol additions — `common/src/protocol.rs`

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameMode {
    FreeRoam,
    CaptureTheArea,
}

pub struct MatchUpdate {
    pub match_id: u32,             // Epoch — client discards stale packets
    pub phase: MatchPhase,
    pub remaining_ms: u32,         // Integer milliseconds, not f32 seconds
    pub blue_score: u32,
    pub red_score: u32,
    pub blue_base_capture_ms: u32, // Progress toward Red capturing Blue base
    pub red_base_capture_ms: u32,  // Progress toward Blue capturing Red base
}

// Sent once at match start, not every tick:
pub struct MatchStartInfo {
    pub match_id: u32,
    pub composition: FleetComposition,
    pub player_team: Team,
}

pub enum Command {
    // ... existing
    SelectGameMode { mode: GameMode }, // Sent from title screen
    SelectShip { entity_type: EntityType },
    StartMatch,  // Client signals ready after ship selection (CTA only)
    PlayAgain,   // Client requests reset() (CTA only)
}
```

**Send cadence:** `MatchUpdate` at 2 Hz during `Playing` (scores change slowly). `MatchStartInfo` is a one-shot on phase transition. **Neither is sent in `FreeRoam` mode.**

#### 8. Unit tests — `server/src/match_state.rs` (mandatory)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_decrements_timer() { /* ... */ }

    #[test]
    fn countdown_transitions_to_playing() { /* ... */ }

    #[test]
    fn playing_transitions_to_ended_at_zero() { /* ... */ }

    #[test]
    fn single_ship_captures_base_in_30_seconds() { /* ... */ }

    #[test]
    fn two_ships_capture_base_in_15_seconds() { /* ... */ }

    #[test]
    fn contested_base_pauses_capture_clock() { /* ... */ }

    #[test]
    fn leaving_base_resets_capture_clock() { /* ... */ }

    #[test]
    fn ship_dying_resets_capture_clock_if_alone() { /* ... */ }

    #[test]
    fn capture_awards_points_and_resets_clock() { /* ... */ }

    #[test]
    fn draw_produces_draw_winner() { /* ... */ }

    #[test]
    fn reset_increments_match_id_and_zeros_scores() { /* ... */ }
}
```

### Client-side Changes

#### 0. Title-screen mode selector — `client/src/ui/game_ui.rs`

Before the ship picker, present a mode selector on the title/spawn screen. Two big tiles:

```
┌─────────────────────────────────────────┐
│              WARSHIPS                   │
│                                         │
│   ┌──────────────┐  ┌──────────────┐    │
│   │              │  │              │    │
│   │  FREE ROAM   │  │  CAPTURE     │    │
│   │              │  │  THE AREA    │    │
│   │   Explore    │  │   5v5 Match  │    │
│   │              │  │              │    │
│   └──────────────┘  └──────────────┘    │
│                                         │
│          Difficulty: [ Easy ▾ ]          │
└─────────────────────────────────────────┘
```

- Free Roam tile → existing spawn flow (difficulty + ship picker → spawn)
- Capture the Area tile → countdown → match (also shows ship picker after)
- Wargame panel styling, 80pt touch targets, Black Ops One labels
- Selected mode is sent to the server via `Command::SelectGameMode { mode: GameMode }` **before** `SelectShip` / `Spawn`

**Mode persistence rule:** Once a mode is chosen on the title screen, it persists for the entire session until the player explicitly returns to the title screen. "Play Again" on the match-end screen stays in the current mode (no mode picker shown) — it calls `reset()` on the existing `MatchState` and re-runs the countdown. To switch modes, the player must hit "Quit to Title" from the pause menu or match-end screen.

#### 1. Pre-match ship picker — `client/src/ui/game_ui.rs`

Replace current spawn screen with ship picker using existing `ShipMenu` + unlocked score:

```rust
<Positioner id="spawn" position={Position::Center}>
    <div style="display:flex;flex-direction:column;gap:24px;align-items:center;">
        {logo()}
        <h2 class="screen-subtitle">{"Choose Your Ship"}</h2>
        <ShipMenu
            entity={None}
            score={u32::MAX}
            allow_npc={false}
            onclick={on_ship_select}
        />
        <button
            class="screen-btn btn-primary"
            disabled={selected_ship.is_none()}
            onclick={on_start_match}
        >{"Start Match"}</button>
    </div>
</Positioner>
```

Wargame button style, same as current Play button.

#### 2. Match HUD — `client/src/ui/match_hud.rs` (new component)

Top-center: countdown timer + team scores. Both bases' capture progress bars visible.

```
┌─────────────────────────────────────────┐
│  BLUE ▌ 287     04:23     142 ▐ RED     │
│  [████░░░░░░] RED BASE  12s / 30s       │  ← Your team capturing
│  [██░░░░░░░░] BLUE BASE 6s / 30s ⚠     │  ← Enemies capturing yours
└─────────────────────────────────────────┘
```

Wargame panel style (`rgba(15,23,42,0.92)` bg, Menlo monospace, colored borders, box-shadow).

#### 3. Countdown overlay — `client/src/ui/countdown_overlay.rs` (new component)

Full-screen "3 / 2 / 1 / FIGHT" during `MatchPhase::Countdown`. Large Black Ops One numbers, each pulses and fades in 1-second intervals.

#### 4. Base + capture ring render — `client/src/game.rs`

In the WebGL render pass, after terrain/ships but before HUD:

1. Draw Blue base at `(0, +500)`: translucent blue fill, blue border ring
2. Draw Red base at `(0, -500)`: translucent red fill, red border ring
3. Draw capture progress ring on each base:
   - Outer ring that fills clockwise based on `capture_progress / 30s`
   - Color is the INVADER team (blue ring on red base when blue is capturing)
   - Ring pulses faster as progress approaches 100%
4. Draw arena boundary at origin radius 1200: thin grey ring

#### 5. Per-player stats tracking — client

Receive player stats in existing player updates. On match end, display results table.

#### 6. Match end results screen — `client/src/ui/match_end_screen.rs` (new component)

```
┌─────────────────────────────────────────┐
│         BLUE TEAM WINS                  │
│                                         │
│     BLUE 287   —   RED 142              │
│                                         │
│  RANK  NAME       SHIP     K  C  PTS    │
│   1   You         Bismarck 7  3  170    │
│   2   Blue Bot 1  Fletcher 4  1   90    │
│   3   Red Bot 2   Kolkata  5  1  100    │
│   ...                                   │
│                                         │
│  [  PLAY AGAIN  ]    [  QUIT  ]         │
└─────────────────────────────────────────┘
```

Play Again button sends `Command::PlayAgain` → server calls `reset()` → fresh match begins with 3-2-1 countdown.

#### 7. Auto-respawn — `client/src/game.rs`

When player dies, show brief "RESPAWNING..." overlay (2s countdown). No ship selection screen — server uses stored loadout.

---

## Implementation Phases

### Phase 1: Match State Machine + Tests + Mode Gating (Day 1)

**Server:**
- Create `server/src/match_state.rs` with full FSM, events, `tick()`, `reset()` ✅ (done, commit 31779ef)
- **Unit tests for all state transitions and capture mechanics** (see test list above) ✅
- Add `team`, `selected_loadout`, `match_stats` to `Player` ✅
- Add `GameMode` enum on server + per-player `game_mode: GameMode` field (default `FreeRoam`)
- Wrap `match_state` in `Option<MatchState>` on the server, only `Some` when any player is in `CaptureTheArea`
- Force team assignment on spawn **only in `CaptureTheArea`** (human = Blue, first 4 bots = Blue, next 5 bots = Red). Free roam bots stay unassigned.
- Structured logging: `info!` on mode change / match start/end, `debug!` on phase changes
- Protocol additions: `GameMode`, `MatchUpdate`, `MatchStartInfo`, `SelectGameMode`, `SelectShip`, `StartMatch`, `PlayAgain` commands
- Hook match tick into server main loop **gated on game mode**

**Client:**
- Title-screen mode selector ("Free Roam" | "Capture the Area") — minimal styling, proof of flow
- Send `Command::SelectGameMode` on tile click before the existing spawn command
- In `CaptureTheArea`: display raw timer + scores (no styling yet) so the data flows end-to-end
- In `FreeRoam`: no HUD changes — game behaves exactly as today

**Deliverable:** Unit tests passing. Free roam still works unchanged. Selecting "Capture the Area" on title screen starts a match, server ticks clock, client shows timer.

### Phase 2: Capture Mechanics + Kill Scoring (Day 2)

**Server:**
- Fixed arena radius 1200 in `world.rs`
- Per-tick base capture logic (with shared team clock)
- Kill scoring hooked into `world_mutation.rs:135` — both team points and per-player stats
- Assign captures to all ships present during a +50 tick (per-player stat tracking)
- Auto-respawn at team base with stored loadout after 2s

**Client:**
- WebGL render pass: draw both bases as circles + capture progress rings
- Match HUD component with wargame styling — scores, timer, capture bars
- "RESPAWNING..." overlay on death

**Deliverable:** Fully playable match. Bases capturable. Scores earned. Per-player stats tracked. Reset not yet wired.

### Phase 3: Ship Selection + Lifecycle (Day 3)

**Client:**
- Replace spawn screen with ShipMenu (unlocked score)
- `Command::SelectShip` + `Command::StartMatch` flow
- Pre-match countdown overlay ("3 / 2 / 1 / FIGHT")
- Match end results screen with sorted per-player stats table
- Play Again button → `Command::PlayAgain`

**Server:**
- Handle `SelectShip`, store `selected_loadout`
- Handle `StartMatch`: transition from `Waiting` → `Countdown`
- Handle `PlayAgain`: call `match_state.reset()`, despawn all boats, respawn bots with new composition, broadcast new `match_id`
- Bot composition selection per match via `FleetComposition::random()`

**Deliverable:** Full loop — ship picker → countdown → match → results → reset → ship picker.

### Phase 4: Polish (Day 4+)

#### From the original polish list
- Edge damage for ships beyond arena radius (verify mk48's existing path triggers against the clamped 1200 radius)
- Audio cues: countdown beeps (3/2/1), match start horn, capture-in-progress chime, capture complete sting, 1-minute warning, match end fanfare
- Playtest balance tuning — capture duration (30s feels long?), kill/capture point ratio (10 vs 50), arena radius, bot difficulty curve
- Spectator-style camera on match end (camera pans to the winning team's base while results overlay fades in)

#### Minimap (promoted from a line to a concrete deliverable)
- Full-arena minimap overlay, bottom-right corner, translucent panel
- Blue dots for allied ships, red diamonds for enemy ships (team-colored markers)
- Both base circles marked with team color
- Current capture progress rendered around each base marker
- Hidden in Free Roam; only renders when `match_update.is_some()`
- New component: `client/src/ui/minimap.rs`

#### Known gaps promoted from the playtest notes
- **Bot "push enemy base" AI** — biggest gameplay gap. Bots currently run mk48's default free-roam AI (roam + attack nearest enemy). They don't understand the CTA objective, so captures only happen when the human player pushes. Add a heuristic: when not in active combat AND the enemy base capture clock is < 20s, steer toward the enemy base. When the enemy is attacking own base, steer home to defend.
- **Capture clock decay instead of hard reset** — the plan's "ship leaves base → clock resets to 0" rule feels brutal when a ship drifts out for a single tick and loses all progress. Change to: clock decays at 2× the capture rate (so a full reset still takes ~15s) while no friendlies are inside. Gives players a grace period to re-enter.
- **Late joiner fleet loadout** — `assign_late_joiners()` assigns a team but forgets to set `selected_loadout` from the current `ai_fleet`. Late-joining bots spawn with random ships instead of fleet-appropriate ones. One-line fix.
- **Fleet display in HUD** — the client never shows what fleet was picked for the current match. Add a small line to the pre-match countdown overlay or the match HUD: "Fleet: Bismarck, Fletcher ×2, Kolkata ×2" or similar.
- **Stale client state on Play Again** — haven't stress-tested repeated Play Again. Verify `match_id` epoch handling actually discards stale packets when it matters. Write an integration test for the reset → countdown → play loop.
- **Ship picker level state reset on Back** — when the player taps Back from ShipPicker, the current level tab isn't preserved if they re-enter. Move level state to parent (Mk48Ui) or wrap it in a shared use_state.
- **Zero-value stat rows in ship detail panel** — the picker shows `Mines 0 / Aircraft 0` for most ships. Cosmetic; hide rows whose value is 0 to tighten the panel.

#### Ordered by impact on feel
1. Bot push-to-base AI — makes bots actually play the objective
2. Capture clock decay — removes the "why did my progress vanish" frustration
3. Audio cues (countdown + match end) — fast wins, big atmosphere bump
4. Minimap — the biggest missing piece of situational awareness
5. Edge damage verification — cheap sanity check
6. Late joiner loadout fix — trivial
7. Everything else — after a proper playtest

---

## File Manifest

### New Files
- `server/src/match_state.rs` — FSM, capture logic, scoring, reset, tests
- `client/src/ui/match_hud.rs` — Timer + scores + capture progress bars
- `client/src/ui/countdown_overlay.rs` — 3-2-1-FIGHT intro
- `client/src/ui/match_end_screen.rs` — Results + stats table + Play Again

### Modified Files
- `server/src/server.rs` — Hook match tick, dispatch events
- `server/src/player.rs` — Add `team`, `selected_loadout`, `match_stats`
- `server/src/bot.rs` — `spawn_for_team()` with composition rotation
- `server/src/world_mutation.rs` — Award team points + stats on kill
- `server/src/world.rs` — Fix arena radius to 1200
- `server/src/world_inbound.rs` — Auto-respawn with stored loadout
- `server/src/world_spawn.rs` — Spawn at team-specific base positions
- `common/src/protocol.rs` — `MatchUpdate`, `MatchStartInfo`, new Commands, `Team`, `MatchPhase`, `FleetComposition`
- `client/src/ui/game_ui.rs` — Replace spawn screen, show match HUD, handle phase transitions
- `client/src/game.rs` — Render base circles + capture rings in WebGL

---

## Resolved Decisions

1. ✅ Match ends at exactly 5:00 (no early-end)
2. ✅ Respawn at team home base (2-second delay, stored loadout)
3. ✅ **Two bases with 30-second capture**, shared team clock, 50 pts per capture
4. ✅ AI rotation: 4 compositions (Battle Line, Destroyer Squadron, Mixed Fleet, Light Raiders)
5. ✅ Submarine Wolfpack deferred to v2 (needs auto-surface rule to be fair)
6. ✅ Player picks any ship before each match (unlocked)
7. ✅ Pre-match 3-2-1 countdown
8. ✅ Per-player stats tracked and displayed on results (kills, captures, ship, points)
9. ✅ Play Again via proper `reset()` method (not server restart)
10. ✅ Arena is fixed at 1200 radius in CTA mode, edge damage beyond
11. ✅ **Free Roam stays as the default mode**, CTA is a second selectable mode
12. ✅ **Title screen mode selector** — user-facing label is "Capture the Area"
13. ✅ Match state is gated behind a `GameMode` enum — `MatchState` only runs when at least one player is in CTA
14. ✅ **Mode persists until return-to-title** — Play Again stays in the same mode, no mode-switch at match end

---

## Risks

| Risk | Mitigation |
|------|------------|
| Capture clock is complex to get right | Unit tests in Phase 1 cover all edge cases (contested, leave, die, multi-ship) |
| mk48's dynamic world radius resists being fixed | Force `target_radius()` return value. Test early. |
| Per-player stat tracking has edge cases (who gets credit for a capture?) | Rule: every friendly ship present at the moment of capture gets +1 to their `captures` stat |
| `reset()` leaves zombie entities | Explicit despawn loop over all boats before respawning. Verify in Phase 3. |
| Protocol changes break client/server sync | Bump protocol version, include `match_id` epoch so clients can discard stale packets |
| Bot AI doesn't understand "push enemy base" objective | Phase 4 tuning — bots may need a heuristic to move toward enemy base when not in combat |
| Bots stacking in their own base earns nothing but feels safe | Phase 4 — bot goal biasing toward enemy base, not defensive camping |

---

## Out of Scope (v1)

- Submarine Wolfpack composition (v2)
- Multiple match modes (just 5-min capture)
- Multiplayer / online / lobbies
- Ranked play / persistent rankings
- Kill assists
- Team chat
- Spectator mode
- Custom team names (always Blue/Red)
- Player-selected team (always Blue)
- Session-level stats across matches ("you've played 12 matches today")
