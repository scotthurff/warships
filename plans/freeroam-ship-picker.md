# feat: Ship picker for Free Roam (pick-your-starting-level)

> **Status:** Planned — 2026-04-13
> **Motivation:** Unify the title-screen flow. Let Free Roam players
> pick their starting ship the same way CTA players do. Starting ship
> = starting level = starting score floor.

## Overview

Today the title flow branches by mode:

- **Free Roam:** Title → pick mode → Continue → **spawn immediately as `EntityType::G5`**.
- **CTA:** Title → pick mode → Continue → **ShipPicker** → Start Game → spawn.

This plan collapses the two paths. Both modes go Title → ModeSelect →
ShipPicker → Start Game. The picked ship determines the player's starting
level in Free Roam: score is seeded to `level_to_score(picked.level)` on
the initial spawn, and normal upgrade progression (earn score, unlock
higher ships) takes over from there.

**Why this works well for kids (per CLAUDE.md):** picking a bigger ship
= easier match — same implicit difficulty mechanic CTA already uses.

## The key design decision

The user framed the open question as: *"not sure how that will affect
points progression — maybe you instead choose your ship and that's your
starting level?"* — which is exactly the right answer. This plan commits
to **"picked ship = starting level floor, progression continues from
there"** over three alternatives:

| Option | Behavior | Why not |
|--------|----------|---------|
| **(chosen) Starting-level floor** | Score seeded to `level_to_score(picked.level)` on first spawn. Upgrades continue upward. Respawn after death uses existing score-diminish + ShipMenu flow. | Matches CTA's "bigger ship = easier" philosophy. Zero-churn integration with existing upgrade / respawn / ShipMenu code. |
| Score stays 0, allow arbitrary ship | Bypass `can_spawn_as` like CTA does via `match_team`. | Breaks respawn. After death, `respawn_score` → 0, ShipMenu offers only Level 1, user is forced into a downgrade cycle they didn't sign up for. |
| Pick ship = fixed respawn ship (CTA-style) | Auto-respawn at the picked ship after death, no ShipMenu. | Kills Free Roam's progression loop, which is the entire point of Free Roam. |

Commit to option 1. The others are documented only so reviewers can
push back on the choice.

## Current state

### Client — two-step branching today

`client/src/ui/game_ui.rs:101-112` — `on_continue`:

```rust
Callback::from(move |_: MouseEvent| {
    if mode == GameMode::CaptureTheArea {
        title_step.set(TitleStep::ShipSelect);  // CTA goes to picker
    } else {
        play_cb.emit(default_alias());          // Free Roam spawns now
    }
})
```

`client/src/ui/game_ui.rs:76-84` — `on_play`:

```rust
gctw.send_ui_event_callback.reform(move |alias| UiEvent::Spawn {
    alias,
    entity_type: selected_ship.unwrap_or(EntityType::G5), // default G5
    game_mode: mode,
})
```

### Server — score-gated spawn

`server/src/world_inbound.rs:80-84` (inside `Spawn::apply`):

```rust
if player.match_team.is_none()
    && !self.entity_type.can_spawn_as(player.score, true)
{
    return Err("cannot spawn as given entity type");
}
```

`can_spawn_as` in `common/src/entity/_type.rs:36` gates on
`level_to_score(data.level) <= score`. Free Roam players have
`match_team = None` so they hit this gate. Their initial `score` is
set by `server/src/server.rs:465-473`:

```rust
free_points: if context.topology.local_arena_id.realm_id.is_temporary()
    || context.topology.local_arena_id.realm_id.is_named()
{
    level_to_score(3)
} else if cfg!(debug_assertions) {
    level_to_score(EntityData::MAX_BOAT_LEVEL)
} else {
    0            // ← public prod: score = 0 → only Level 1 ships spawnable
},
```

So on prod, a Free Roam player's first-spawn score is 0 and they can
only spawn as a Level 1. That's why the current Free Roam path just
force-spawns as `G5` (a Level 1 corvette).

## Fix

Three small changes, one PR.

### 1. Server — seed score on initial spawn

`server/src/world_inbound.rs` — inside `Spawn::apply`, before the
`can_spawn_as` gate, bump the player's score up to the picked ship's
floor **if this is their first spawn of the session**:

```rust
// Free Roam ship-picker: the user explicitly chose this ship on
// the title screen. Seed their score to the ship's level floor so
// can_spawn_as accepts it and the upgrade overlay shows the right
// goals. Gated on is_spawning (first-spawn, not respawn) so
// post-death respawns keep the existing respawn_score semantics.
//
// `alias.is_some()` distinguishes title-screen Spawn from
// client/src/game.rs:1824 Respawn, which passes alias: None.
if player.match_team.is_none() && self.alias.is_some() {
    let floor = level_to_score(self.entity_type.data().level);
    player.score = player.score.max(floor);
}
```

Signal gating — `self.alias.is_some()` — is the existing, reliable
distinction between title-spawn and mid-session respawn. Already used
in `Spawn::apply:51` for alias-setting. Not inventing a new field.

`.max(...)` (not `=`) preserves higher existing scores — relevant for:
- Debug builds where `free_points = level_to_score(MAX_BOAT_LEVEL)`.
- Any future code path that pre-populates score before the first spawn.

### 2. Client — always route through ShipPicker

`client/src/ui/game_ui.rs:101-112` — `on_continue` always advances to
`TitleStep::ShipSelect`, regardless of `selected_mode`:

```rust
let on_continue = {
    let title_step = title_step.clone();
    Callback::from(move |_: MouseEvent| {
        title_step.set(TitleStep::ShipSelect);
    })
};
```

No more mode branch. `play_cb` is dropped from this closure (now only
used by `on_start_from_picker`). The Spawn command already carries
`game_mode` at `client/src/ui/game_ui.rs:82`, so CTA vs Free Roam is
still differentiated downstream; the title flow just stops caring.

### 3. Client — ShipPicker defaults + copy-check

`client/src/ui/ship_picker.rs` — currently header says "Select Your
Ship" (generic). No change needed. Level nav, detail panel, Start Game
button all apply cleanly to Free Roam.

One copy consideration: the picker today is unambiguously CTA-themed
in context (reached only via CTA tile). In Free Roam context it's
still correct, but we may want a subtle mode tag somewhere — e.g., a
small "FREE ROAM — CHOOSE YOUR STARTING SHIP" subheader vs
"CAPTURE THE AREA — CHOOSE YOUR LOADOUT". Decision deferred to
implementation — if the existing UI reads cleanly for both, skip.

### 4. Remove the Free-Roam `Start Game` fallback path

`client/src/ui/game_ui.rs:298-302` — the ModeSelect-step Continue
button currently shows different labels:

```rust
if *selected_mode == GameMode::CaptureTheArea {
    {"Continue >"}
} else {
    {"Start Game"}
}
```

After this change, the button always means "Continue to ship picker."
Label simplifies to always `"Continue >"`.

## Acceptance

- [ ] Free Roam title flow: ModeSelect → Continue → ShipPicker → pick
      Level N ship → Start Game → spawn successfully in Free Roam.
- [ ] Initial Free Roam spawn ship matches the picked entity type.
- [ ] Initial Free Roam score equals or exceeds `level_to_score(picked.level)`.
- [ ] After dying in Free Roam, the existing RespawnOverlay + ShipMenu
      flow is unchanged. `respawn_score` semantics preserved.
- [ ] CTA title flow unchanged end-to-end (Continue → ShipPicker →
      Start Game → match starts, Blue slot 0, 4+5 bots assigned).
- [ ] The `ShipPicker` component works identically in both modes
      (no mode-specific branch inside the picker).
- [ ] `can_spawn_as` no longer rejects high-level Free-Roam title-spawns.

## Test plan

**Unit (existing upgrade/level math):** no new tests required — this
plan only *reuses* `level_to_score`, doesn't change it. The seed logic
in `Spawn::apply` is narrow enough that a manual test covers it.

**Manual smoke test** — prod, iPad and desktop:

1. Hard-refresh, pick **Free Roam → Continue**.
2. Expect: ship picker renders (the same one CTA uses).
3. Pick a Level 8 ship. Start Game.
4. Expect: spawn in the world as the picked Level 8 ship, not G5.
5. Check the upgrade overlay — it should show Level 9 as the next goal
   (score already at Level 8 floor).
6. Die (sail into enemy fire). RespawnOverlay appears with the normal
   ShipMenu (not the full picker). Pick a Level 7 ship. Verify respawn
   works (existing behavior).
7. Repeat steps 1–2 picking Level 1 Lürssen. Verify: spawn as Level 1,
   upgrade overlay shows Level 2 goals (no score change since score
   was already ≥ level_to_score(1) = 0).
8. Switch to CTA mode: verify the full CTA flow still works
   (countdown, 5-minute match, capture mechanics, match-end, Quit
   to Title, re-entry — regression of the recent fix).

## Files

| File | Change | Rough LOC |
|------|--------|-----------|
| `server/src/world_inbound.rs` | Score seed in `Spawn::apply` | ~6 |
| `client/src/ui/game_ui.rs` | Drop mode branch in `on_continue`; simplify button label | ~8 |
| `client/src/ui/ship_picker.rs` | (Optional) mode subheader copy | 0–5 |

~20 LOC total. One commit.

## Edge cases considered

- **Debug builds** (`free_points = level_to_score(MAX)`): picking a
  lower-level ship doesn't downgrade score thanks to `.max()`. ✓
- **Rank bump** at `Spawn::apply:55-57`
  (`if rank >= Rank3 { score.max(level_to_score(2)) }`): still applies
  after our bump — if the player's rank pushes them to a higher floor
  than the picked ship's level, the higher floor wins. Compatible. ✓
- **Respawn after death**: `self.alias.is_none()` for Respawn commands
  (`client/src/game.rs:1824-1828`), so our score seed doesn't fire on
  respawn. Existing respawn flow untouched. ✓
- **CTA paths**: gated on `player.match_team.is_none()` so CTA players
  (match_team = Some) never hit the seed logic. ✓
- **`selected_ship == None` at Start Game**: the picker's Start Game
  button is already `disabled={!has_selection}`
  (`ship_picker.rs:143`), so this can't happen. ✓

## Non-goals / deferred

- **Changing Free Roam respawn to use the full picker** instead of
  `ShipMenu`. Out of scope — respawn already has a working picker,
  just a smaller one. Changing it would make deaths feel heavier,
  which hurts the kid-friendly pacing.
- **Difficulty presets** (Captain / Admiral / Fleet Cmdr on the
  ModeSelect step) — untouched. Those control bot difficulty and
  apply to both modes.
- **Removing the mode picker entirely** and defaulting to Free Roam
  or CTA — no. Both modes stay first-class.
- **Serving the `last_spawn_entity` as the picker's default
  selection** on subsequent title-visits — defer. Today the user
  always starts fresh. Revisit if users complain.

## References

- `server/src/world_inbound.rs:28-84` — `Spawn::apply`, the score gate
- `server/src/server.rs:465-473` — `free_points` seeding on server boot
- `common/src/util.rs:9-23` — `level_to_score` / `score_to_level`
- `common/src/entity/_type.rs:36` — `can_spawn_as`
- `client/src/ui/game_ui.rs:40-126` — `TitleStep`, title flow callbacks
- `client/src/ui/ship_picker.rs` — the component being reused
- `client/src/ui/ship_menu.rs` — the respawn picker (unchanged)
- `client/src/ui/respawn_overlay.rs` — Free Roam respawn (unchanged)
- CLAUDE.md — "Bot difficulty: Easy mode is the default. Tune for
  ages 5-10" and "picking a bigger ship = easier"

## Open questions

1. **Picker subheader per mode?** Worth a copy tweak, or does "Select
   Your Ship" read cleanly for both? Defer to implementation — if
   confused during manual test, add.
2. **Should the Free Roam ship picker default to a mid-level ship**
   (e.g., Level 5) instead of nothing? Today the CTA picker has no
   default; the user has to tap. For a kid, "Level 5" might be a
   kinder default than an intimidating Level 1 grid. Skip for now;
   measure if kids get stuck.
3. **Future: save last-played ship?** LocalStorage-backed default.
   Not in this scope; revisit if the flow feels repetitive.
