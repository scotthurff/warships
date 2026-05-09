# WARSHIPS — Decision Log

## 2026-05-09: Remove G5 fallback in `on_play` — spawning requires explicit pick

**Context:** The ship-picker auto-dismiss bug recurred despite the
`start_requested` gate added in `aefe578`. Symptoms identical to the
prior 2026-05-05 fix: picker disappears ~2 s after Continue, player
spawns as G5 with no input. Investigation found the gate logic was
correct in isolation — the recurring foothold was
`selected_ship.unwrap_or(EntityType::G5)` in `on_play`. Any path
firing `on_play.emit()` while no ship was picked produced a working
G5 spawn, regardless of whether the surrounding gates were right.

**Decision:** Remove the G5 fallback. `on_play` is now a
`Callback<PlayerAlias>` that is a no-op when `selected_ship` is
`None`. Spawning requires an explicit pick by construction, not by
gating. Plus belt-and-suspenders: idempotent `on_start_from_picker`,
`selected_ship.is_some()` added to the retry-interval deps, and a
`start_requested` reset in the in-match effect.

**Alternatives considered:**
- *Tighten the existing gate further* — same family of fix as 2026-05-05,
  same failure mode if any path we haven't enumerated leaks through.
- *Add a "committed_ship: Option<EntityType>" cell that the retry uses
  instead of live `selected_ship`* — semantically cleaner (retry
  spawns the ship the user committed to, not the live selection), but
  larger blast radius and not needed to fix the immediate bug.
- *Server-side `SpawnFailed` wire message* — already documented as
  rejected in `cta-spawn-stuck-ship-picker.md`, invasive into the mk48
  descent.

**Consequences:** Spurious `on_play.emit()` paths are now harmless —
they dispatch nothing. The retry still fires every 2 s after a real
Start Game tap to recover from server "no spawn slot" failures. The
class of bug "user lands in a default ship without input" is no
longer reachable from the client. Any future addition to the spawn
flow that relies on a default ship type will fail loudly instead of
silently spawning. Documented in
`docs/solutions/ui-bugs/ship-picker-g5-fallback-foothold-2026-05-09.md`.

## 2026-04-09: Tap-to-target already works via kodiak touch→mouse conversion

**Context:** Needed tap-to-target aiming for touch devices. Investigated game.rs aim_target pipeline.

**Decision:** Tap-to-target works out of the box. Kodiak converts touch events to mouse position internally (`context.mouse.view_position`). When you tap the screen, turrets aim at that world position. No code changes needed for basic tap aiming.

**What's NOT done:** Target locking (turrets track a specific ship as it moves). This requires maintaining a locked entity ID and updating aim_target each frame to that entity's position. Deferred to Phase 3.

**Consequences:** Touch aiming works for v1 — tap to set aim direction, tap FIRE to shoot. Not as polished as WoWS Blitz lock-on, but functional.

## 2026-04-09: Difficulty selector deferred — Easy mode is default

**Context:** The plan called for Captain/Admiral/Fleet Commander difficulty buttons on the spawn screen. But bot difficulty params (aggression, aim_bias, speed, fire rate) live in the server's bot.rs and can't be changed per-client at runtime without a server-side flag.

**Decision:** Make Easy mode the default (already done — bot AI tuned with lower aggression, worse aim, slower speed, capped level 5). Defer the difficulty selector UI. When we add it, it'll need a server CLI flag or URL parameter.

**Consequences:** All players get Easy difficulty. Good for v1 kid-focused testing. Difficulty selector becomes a Phase 3 task.

## 2026-04-08: Pivot from custom 3D to mk48.io fork

**Context:** Spent several hours building a custom 3D naval game — first in Three.js (browser), then SceneKit (native). Both had fundamental rendering issues: the Three.js Water shader created ugly reflection artifacts when ships intersected the water plane, and SceneKit had model loading / wave shader issues. Ship models were also a problem — free models were either too high-poly (294K faces, z-fighting) or too low-poly with no textures.

**Decision:** Fork mk48.io (open-source Rust/WASM naval combat game) instead of building from scratch. It already has 45 ships, weapons, bot AI, water rendering, and runs in any browser.

**Alternatives considered:**
- Custom Three.js game (abandoned — water/ship clipping unsolvable in WebGL)
- Custom SceneKit/Metal native app (abandoned — model pipeline issues, slower iteration)
- Unity BoatAttack fork (good water but no combat, C# not Rust)
- Godot OceanFFT + Open RTS combo (too much assembly required)

**Consequences:** Game is 2D top-down instead of 3D. Visual style is different from WoWS but gameplay is solid. Rust/WASM stack instead of TypeScript. AGPL license requires open-sourcing if distributed beyond personal use.

## 2026-04-08: Branding — WARSHIPS with Black Ops One font

**Context:** Need a distinct identity from mk48.io.

**Decision:** Name: "WARSHIPS". Logo uses Black Ops One font (reused from conflict-of-nations game). Subtitle: "NAVAL COMBAT". HUD/controls use Menlo/SF Mono monospace (not the logo font — too heavy for buttons).

**Alternatives considered:** Keeping mk48 branding (rejected — want our own identity).

**Consequences:** Black Ops One woff2 added to client/data/. Logo replaced in logo.rs with simple HTML text instead of SVG.

## 2026-04-08: Replace kodiak SpawnOverlay with custom spawn screen

**Context:** Kodiak framework's SpawnOverlay component has hardcoded nickname field, "Play with friends" button, and social links that can't be disabled via props.

**Decision:** Replaced SpawnOverlay entirely with our own Positioner + logo + Play button. Also replaced splash_links (which injected Feedback/Privacy/Terms) with plain HTML links for Help and Ships only.

**Alternatives considered:** CSS/JS hacks to hide elements (tried first, unreliable — text matching broke other elements).

**Consequences:** We no longer use SpawnOverlay or splash_links from kodiak. Our spawn screen is simpler but fully under our control.

## 2026-04-08: Touch controls — WoWS Blitz style layout

**Context:** mk48.io is mouse-only (hold to steer, click to fire, cursor = aim). Need touch controls for iPad.

**Decision:** WoWS Blitz-style layout: bottom-left rudder buttons (L/R) + speed buttons (STOP/1/2/FULL), bottom-right FIRE button (80px red circle). Separate torpedo button planned. Tap-to-target for aiming planned.

**Alternatives considered:** Virtual joystick (Roblox style), dual-stick (Diep.io style). Chose discrete buttons because WoWS Blitz proves they work for naval combat and they're simpler for kids.

**Consequences:** Touch controls are a Yew component (touch_controls.rs) communicating via UiEvent to game.rs. Works on desktop too (clickable) — will add touch-only detection later.

## 2026-04-08: Ship models — deferred

**Context:** Spent significant time evaluating ship models. 294K STL rips are unusable. 19K low-poly models work but have no PBR textures. mk48 uses 2D sprites so this is moot for now.

**Decision:** Defer 3D model sourcing. The 2D sprite approach from mk48 works fine. If we ever want 3D, the research is in warships-old-prototype/plans/.

**Alternatives considered:** Buying game-ready models ($15-50 each), AI generation (Meshy.ai), commissioning artists.

**Consequences:** No 3D model work needed. Sprite-based rendering is simpler and performs well on iPad.

## 2026-04-15: CTA bot AI overhaul — flow fields, steering, arena expand, state machine

**Context:** CTA bots consistently beached themselves on terrain, drove in circles near bases, and never reached the enemy base. Five iterations of tuning the Reynolds-boids force aggregation (weights, springs, look-ahead) each failed a specific gate but revealed a new failure mode. The pattern was architectural, not a bug in any one constant.

**Decision:** Shipped a five-layer rebuild of the bot AI and arena layout on the `flow-field-pathfinding` branch:

1. **Flow-field pathfinding** (`server/src/bot_pathfinder.rs`): Dijkstra-from-goal integration field + per-cell flow vectors, built once per CTA match start. Replaces the per-tick direct-vector pull toward the enemy base. Routes around terrain automatically. `INFLATION_CELLS = 2` and a two-zone goal-clear (force-open 3-cell inner + undo-inflation 14-cell outer) guarantee reachability near the bases.

2. **Non-holonomic steering layer** (Phase 3b, `server/src/bot.rs` outer update): 4-sample forward trace along the ship's projected arc, turn-budget throttle that scales velocity to what the ship's turn rate can execute in the look-ahead window. Prevents bots from committing full-speed headings they can't physically steer.

3. **Arena expand + terrain sparsen** (Phase 4, `server/src/cta_terrain.rs`, `match_state.rs`): base distance doubled from 1000 m → 2000 m (bases at 0, ±1000); arena radius 1500 → 3000 m; 70% of blocking land inside the base-to-base corridor flattened at match start via a MurmurHash3 scatter. Deliberately preserves 30% of islands for visual texture — avoids repeating the reverted "flat blue disk" of commit ae1026a.

4. **State machine + Pure Pursuit path follower** (Phase 6, `server/src/bot_behavior.rs`, new ~700 LOC module): replaces per-tick `movement = Σ forces` with a 4-state FSM (`Spawning`, `Transiting { committing }`, `Engaging`, `Defending`) + `trace_path` multi-waypoint path + velocity-adaptive carrot. Asymmetric hysteresis on every transition (40-tick Engaging hold-out, 10s/3s Defending thresholds, top-3-closest defender rule with team-size <3 short-circuit). Replaces Reynolds 1987 boids with 2006-era goal-directed-agent AI.

5. **Mixed-team rushers + fog-off + spawn retry**:
   - `prefers_rush: bool` on Bot (~35% true) gives a fraction of bots rusher priority: when within 700 m of the enemy base, Engaging is bypassed and Committing mode is forced. Other ~65% engage in midfield combat as before. Produces naturally mixed team behavior.
   - CTA fog-of-war removed for the HUMAN player only (`world_outbound.rs`). Early version accidentally applied to bots too, giving them unlimited-range `closest_enemy` and causing out-of-weapon-range fire. Gated to `is_human` after measurement.
   - Client-side spawn retry (`client/src/ui/game_ui.rs`): re-emits the Spawn command every 2 s if `UiStatus::Spawning` persists while the ship-picker is showing. Handles the server's silent spawn failures (slot clipping bot / sparsen island) without server-side protocol changes.

**Alternatives considered and rejected:**

- *Tuning one more constant* — the Phase 5 objective-weight bump (`in_combat ? 1.5 : 6.0` → `6.0`) was shipped and measured: `terrain_deaths` regressed 12 → 83, `enemy_base_reached` stayed 0. Reverted. Established the rule: when a plan's hard-stop criterion fails, we stop and restructure.
- *Per-ship-class flow fields* — one flow field per level × per goal = 20 fields, dismissed as memory-for-complexity trade that didn't buy commensurate fidelity at ship scale.
- *Completely flat arena* — reverted earlier (commit `b32bed0`) for producing a visually barren blue circle. The 70/30 sparsen is the compromise.
- *Hand-scripted island geometry* — deferred. If the 70/30 sparsen proves insufficient we'd try this before giving up, but the current measurement (terrain_deaths=4 in live test) suggests it's not needed.
- *Client-side spawn retry alternative: server SpawnFailed wire message* — would require protocol changes into the mk48 descent, invasive. Client retry handles every flavor of server spawn failure including ones we haven't identified.

**Consequences:**

- Bot behavior is measurably better: terrain_deaths dropped from 376 (original failure match) to 4-10 per match depending on configuration. Throttle-rate dropped from 80% to 4-7%. State transitions stay at 46-137 per match (expected 50-150, gate 200 — healthy, no flap).
- The primary `enemy_base_reached ≥ 3/team` gate never passed in measurement, but bots visibly reach the base vicinity (observed at y ≈ enemy base ± 350 m). The 250 m capture-ring threshold is strict; bots engage enemies outside the ring rather than rushing blind. Rushers partially address this.
- New runtime counters on `World` and `Server` (all `#[cfg(debug_assertions)]`): `cta_bot_terrain_deaths`, `cta_bots_enemy_base_reached`, `cta_bot_ticks_throttled`, `cta_bot_alive_ticks`, `cta_bot_state_transitions`. Logged on `MatchEvent::MatchEnded`.
- The 400+ LOC state-machine module is now the primary bot-decision code path; the old boids aggregation remains ONLY as the Free Roam branch in `bot.rs` (CTA is explicitly the state-machine path).
- `decisions.md` and `plans/` directory gained multiple plan artifacts documenting each phase's theory, acceptance gate, and (for Phase 5) its measured falsification. The plans are intentionally left in-tree as history, not deleted after shipping.
- Score parity: Blue vs Red CTA matches with human + rushers ran balanced (110/90, 110/70, 170/150 final-UI scores). Before this work, bots would respawn and re-die near spawn without ever pushing — matches were lopsided or stalemated.

**What's left open:**

- `enemy_base_reached` strict gate failure — noted, not tuned further. Could be addressed by loosening threshold to 400 m or by changing the game (shorter base distance, different objective) but both are separate decisions.
- Client/server match-end score discrepancy: server logs score at `MatchEnded` event fire; client UI keeps incrementing for late kills that land after the event. Noted, not fixed — cosmetic.
- Human visibility of bots is working. The inverse (if we ever add network multiplayer) — making humans visible to other humans — is untouched.
