# WARSHIPS — mk48.io Adaptation Plan

## What We're Doing

Fork mk48.io (Rust/WASM naval combat game, 45 ships, bot AI, already runs in browser) and adapt it into WARSHIPS — a kid-friendly single-player naval game for iPad and desktop.

**Repo:** `/Users/scotthurff/Repos/mk48`
**Tech:** Rust server + Rust/WASM client (Yew framework for UI, WebGL for rendering)
**Runs at:** `https://localhost:8443/`

---

## Mods (Ordered by Impact)

### 1. Touch Controls + Weapon Aiming

The #1 blocker for iPad play. Current controls are mouse-only (hold to steer, click to fire, cursor position = aim target).

**Where to modify:**
- `client/src/game.rs` — main input handling in `peek_keyboard()` (line 426), `peek_mouse()` (line 469), and `tick()` (lines 1430-1570)
- The kodiak engine already has a `joystick_controller.rs` component and converts touch→mouse internally
- `context.mouse.touch_screen` boolean already exists (game.rs line 1533)

**What to build:**

**Mirror World of Warships Blitz touch layout:**

```
+------------------------------------------------------------------+
|  [Minimap]                          [Scope/Zoom]  [Ammo Type]    |
|                                                                   |
|                                                                   |
|                     (Game View — tap to target)                   |
|                                                                   |
|                                                                   |
|  [Rudder]     [Speed]                    [Torpedo]  [FIRE]       |
|  (left/right  (stop/1/2/full             (launch)   (main guns)  |
|   arrows)      buttons)                                           |
+------------------------------------------------------------------+
```

**Navigation (bottom-left, WoWS style):**
- **Rudder:** Left/right arrow buttons for steering (NOT a joystick — WoWS uses discrete rudder)
- Maps to existing `direction_target` by adjusting heading ± a fixed turn rate
- **Speed:** 4 speed buttons in a vertical stack (Stop / 1/4 / 1/2 / Full)
- Maps to existing `velocity_target` (0, 0.25, 0.5, 1.0 of max speed)
- All buttons: 80pt minimum, high contrast

**Targeting + Aiming (center screen):**
- **Tap anywhere on the game view** = set `aim_target` to that world position. Turrets auto-rotate toward it.
- **Tap an enemy ship** = lock onto that ship. Turrets track it automatically as it moves.
- Target lock indicator: ring around locked ship (WoWS style)
- The existing `find_best_armament()` in `client/src/armament.rs` (line 20) already selects the best weapon for a given aim target — reuse this
- Auto-aim assist on Easy: widen the acceptable angle cone (currently 30° for shells at line 82)

**Firing (bottom-right, WoWS style):**
- **Main guns button** (80pt, bottom-right): Fires primary weapons at aim target
- **Torpedo button** (80pt, above main guns): Launches torpedoes in the aimed direction
- Each button shows cooldown timer (radial fill overlay, WoWS style)
- Greyed out when reloading, glows when ready
- Currently left-click fires (game.rs line 1556) — buttons trigger the same code path

**Weapon/ammo selection (top-right, WoWS style):**
- Ammo type toggle (HE/AP equivalent if applicable to mk48's weapon system)
- If mk48 doesn't have ammo types, this becomes weapon group selector (guns / torpedoes / missiles)
- Small buttons (60pt), secondary to the fire buttons

**Scope/Zoom (top-right):**
- Zoom toggle or pinch-to-zoom for aiming at distant targets
- WoWS has a binocular/scope mode — we can map this to camera zoom

**Safari touch prevention (critical):**
- Add gesture prevention JS in `client/index.html` (prevent pinch-zoom, double-tap zoom)
- Safari doesn't fully support `touch-action: none` — needs JS workaround

---

### 2. Strip Multiplayer / Simplify for Single-Player

**What to remove from `client/src/ui/game_ui.rs`:**
- `splash_social_media` (line 102)
- `splash_links` (line 103)
- `splash_sign_in_link` (line 104)
- `splash_nexus_icons` (line 106)
- `LeaderboardOverlay` (lines 107-111)
- `ChatOverlay` (lines 90-95)
- `TeamOverlay` (lines 75-85)

**What to change in `common/src/lib.rs`:**
- `name: "WARSHIPS"` (was "Mk48.io")
- `domain: "localhost"` (was "mk48.io")
- `geodns_enabled: false` (was true)

**Server (`server/src/server.rs`):**
- The server already spawns bots automatically — no changes needed for single-player
- Bots fill the arena whether or not other players connect
- Just run the server locally and connect one browser tab

**What stays:** The entire game loop, physics, combat, spawning — it all stays. We're just removing the social/multiplayer chrome around it.

---

### 3. Bot Difficulty Tuning

**File:** `server/src/bot.rs`

**Current bot parameters (line 26-53):**
- `aggression`: 0 to 0.1 (most bots barely fight)
- `aim_bias`: random offset up to 10m
- `steer_bias`: random angular offset
- `level_ambition`: 1-10 (biased low)

**For Easy ("Captain Mode"):**
- Reduce `MAX_AGGRESSION` from 0.1 to 0.03 (bots mostly patrol, rarely attack)
- Increase `aim_bias` max from 10m to 30m (bots miss a lot more)
- Cap `level_ambition` at 3 (bots stay in small ships)
- Bot speed: reduce from `0.8 * max` to `0.6 * max`
- Fire probability: reduce from `aggression.min(1.0)` to `aggression.min(0.3)`

**For Medium ("Admiral Mode"):**
- Keep current values (they're already moderate)

**For Hard ("Fleet Commander Mode"):**
- Increase `MAX_AGGRESSION` to 0.2
- Reduce `aim_bias` to 3m
- Remove the "no sinking level 1" mercy rule (line 234)
- Allow `level_ambition` up to 8

**Implementation:** Add a `difficulty: Difficulty` enum to the server config, passed via CLI flag or URL parameter. Bot::new() reads this to set parameter ranges.

---

### 4. Rebrand as WARSHIPS

**Files to change:**

| File | What to change |
|------|---------------|
| `common/src/lib.rs:18-26` | `name: "WARSHIPS"`, `game_id: "Warships"`, `domain: "localhost"` |
| `client/index.html:11-19` | `<title>WARSHIPS</title>`, meta tags, theme-color |
| `client/data/manifest.json` | `"name": "WARSHIPS"`, short_name, icons |
| `client/src/ui/logo.rs` | Replace SVG with "WARSHIPS" text in Black Ops One font |
| `client/src/ui/translations/about/en.md` | New game description |
| `client/src/ui/translations/help/en.md` | Updated help text for touch controls |
| `assets/branding/logo.svg` | New logo |

**Font:** Black Ops One (already in the warships repo at `black-ops-one.woff2`). Load via @font-face in `client/index.html`. The logo.rs SVG can be replaced with a simple Yew `<h1>` styled with the font.

**Color scheme:** Keep the naval blue (`#00487d`) — it works.

---

### 5. Enlarge UI for Touch

**Files to change:**

| File | What | Current | Target |
|------|------|---------|--------|
| `client/src/ui/status_overlay.rs:32-36` | Weapon buttons | `min-width: 5rem` | `min-width: 80px` on touch |
| `client/src/ui/ship_menu.rs:47-85` | Ship selection grid | `grid-gap: 1rem` | Larger cards, bigger tap targets |
| `client/src/ui/upgrade_overlay.rs` | Upgrade bar/buttons | Default sizes | 80pt minimum buttons |
| `client/src/game.rs` (WebGL HUD) | Health bars, direction indicators | Small | Scale up 1.5x on touch |

**Approach:** Detect touch via `context.mouse.touch_screen` or `is_mobile()`. Apply a CSS custom property `--hud-scale: 1.5` on touch devices. Use this in all `css!()` macros for sizing.

---

### 6. Simplify Ship Progression

**Current:** 10 levels, score-based exponential unlock. Overwhelming for kids.

**For v1:** 
- Keep all 45 ships available but reduce to **3 tiers** visible in the UI:
  - Tier 1 (levels 1-3): "Patrol Boats" — small, fast, simple
  - Tier 2 (levels 4-6): "Warships" — medium, balanced
  - Tier 3 (levels 7-10): "Capital Ships" — big, powerful
- Auto-upgrade: when score threshold is met, offer ONE upgrade choice (not the full grid of options)
- Or: start with all ships unlocked, no progression. Pick your ship and fight.

**Where:** `common/src/entity/_type.rs:69-82` (`upgrade_options()`) and `client/src/ui/ship_menu.rs`

---

## Build Order

**Phase 1: Strip + Rebrand (day 1)**
- Remove multiplayer UI from game_ui.rs
- Change branding in lib.rs, index.html, manifest.json, logo.rs
- Update help/about text
- Verify it still builds and runs

**Phase 2: Touch Controls (days 2-4)**
- Investigate kodiak's existing joystick_controller.rs — may already work
- Add floating joystick overlay for movement
- Add tap-to-target for aiming
- Add fire button (80pt)
- Safari gesture prevention
- Test on real iPad

**Phase 3: Difficulty Tuning (day 5)**
- Add difficulty enum
- Tune bot parameters for Easy/Medium/Hard
- Add difficulty selector to spawn screen

**Phase 4: UI Scaling (day 6)**
- Enlarge all touch targets
- Scale HUD elements on touch devices
- Simplify ship menu to 3 tiers

**Phase 5: Polish (day 7)**
- Black Ops One font for title
- Adjusted color scheme if needed
- Final iPad testing

---

## Key Files Reference

| System | File | Key Lines |
|--------|------|-----------|
| Input handling | `client/src/game.rs` | 426 (keyboard), 469 (mouse), 1430-1570 (tick/movement/aiming) |
| Weapon auto-select | `client/src/armament.rs` | 20-105 (find_best_armament) |
| Main UI | `client/src/ui/game_ui.rs` | 34-114 (full component, social/login/leaderboard here) |
| Weapon HUD | `client/src/ui/status_overlay.rs` | 28-165 (bottom bar, weapon buttons) |
| Ship menu | `client/src/ui/ship_menu.rs` | 42-196 (ship selection grid) |
| Upgrade system | `client/src/ui/upgrade_overlay.rs` | 22-78 (progress bar, upgrade trigger) |
| Logo | `client/src/ui/logo.rs` | 6-100 (inline SVG) |
| Bot AI | `server/src/bot.rs` | 26-53 (params), 75-200 (behavior), 330 (fire prob) |
| Game constants | `common/src/lib.rs` | 16-26 (name, domain, game_id) |
| Ship definitions | `common/src/entity/_type.rs` | 157-1802 (all 45 ships) |
| Score/level math | `common/src/util.rs` | 9-13 (level_to_score formula) |
| HTML entry | `client/index.html` | 10-19 (viewport, meta, title) |
| PWA manifest | `client/data/manifest.json` | 1-57 (branding) |

---

## Risks

| Risk | Mitigation |
|------|------------|
| Kodiak engine is opaque (external dep) | It's open source at github.com/softbearstudios/kodiak. Clone and read if needed. |
| Touch controls hard to get right in Rust/Yew | Kodiak already has joystick_controller.rs — start there |
| iPad Safari WebGL performance | mk48 is already optimized for browser. Test early. |
| AGPL license for distribution | Personal/family use is fine. Open-source the fork if distributing more broadly. |
| Rust learning curve | The mods are mostly UI (Yew/HTML) and config changes, not deep Rust |
