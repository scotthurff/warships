# WARSHIPS — Decision Log

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
