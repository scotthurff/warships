# fix: CTA stuck-state after Quit to Title

> **Status:** Planned — 2026-04-13
> **Reported from:** Live prod playtest
> **Severity:** High — blocks replay after any multi-match session
> **Review:** DHH / Kieran / Simplicity (2026-04-13, all convergent)

## Problem

After playing N Capture-the-Area matches in a row, clicking **Quit to Title**,
and re-selecting CTA from the title, the game soft-locks on the ship picker:
Start Game not tappable, minimap showing stale bot ships.

**Root cause:** `handle_quit_to_title` at `server/src/server.rs:119` doesn't
actually tear down the match. It only mutates the quitting human's fields.
Everything else — `match_state.phase` stays `Ended`, bots keep their team
assignments and boats, client picker state persists — is downstream of that
single omission. Next CTA entry then routes into `assign_late_joiners`
(which works correctly but on dirty data) and the match never leaves `Ended`,
so `MatchEndOverlay` (z-index 9999) keeps rendering over the ship picker and
swallows taps.

**Repro:**
1. Play 3 CTA matches via Play Again
2. Final match-end screen → **Quit to Title**
3. Title → Capture the Area → Continue
4. Ship picker appears, Start Game disabled, minimap visible with bot ships

## Fix

Two small changes, one PR.

### Server — `handle_quit_to_title` does a real teardown

`server/src/server.rs:119` — full rewrite. Also adds a named
`MatchState::reset_to_waiting()` method (not `MatchState::new()`, see Notes).

What it must do:
1. Remove every alive boat owned by a team-assigned player (`world.remove`
   with `DeathReason::Unknown` — matches `handle_play_again:87` pattern so
   mk48's boat-kind debug_assert doesn't fire).
2. Clear `match_team`, `match_slot`, `selected_loadout`, `match_stats` on
   every team-assigned player. Set their `status = Spawning`, `flags = default`.
3. Flip the quitting human's `game_mode = FreeRoam`.
4. Call `self.match_state.reset_to_waiting()`.
5. Delete the buggy `flags.left_game = true; … = false;` toggle on
   `server/src/server.rs:129-132` (subsumed by the rewrite).

### Server — add `MatchState::reset_to_waiting`

`server/src/match_state.rs` — new method sibling to `reset()`:

```rust
/// Tear down the current match and return to Waiting. Preserves
/// match_id monotonicity so clients discard stale MatchUpdates.
/// Unlike reset() (→ Countdown, used by Play Again), this leaves
/// the match dormant until a player re-enters CTA mode and the
/// tick loop's `if phase == Waiting` path picks it up.
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
```

The `match_id.wrapping_add(1)` is load-bearing. `MatchState::new()` resets
`match_id` to 1, which collides with in-flight `MatchUpdate`s the client may
still be holding — a client with `match_id: 7` in state would see a new match
at `match_id: 1` and have no signal the prior state is stale. This is latent
today and this plan fixes it pre-emptively.

### Client — reset title-screen state when match ends

`client/src/ui/game_ui.rs:63` — add a `use_effect_with` in `mk48_ui` that
watches `props.match_update.as_ref().map(|m| m.match_id)`. On change
(including `Some(n) → None`), reset `selected_mode`, `selected_ship`,
`title_step`.

Why this dependency: match_id changes on exactly two events — Play Again
(`reset()`) and Quit to Title (`reset_to_waiting()`). Play Again keeps
`match_update` as `Some(new_match_id)` → effect fires, resets cells, but
since the user is in `UiStatus::Dead/Respawning` the title screen isn't
rendered anyway (render is idempotent). Quit to Title drops
`match_update` to `None` → effect fires, resets cells, user lands on
`ModeSelect` cleanly on next render.

## Acceptance

- [ ] After 3×Play Again → Quit to Title → re-enter CTA: ship picker has
      no pre-selected ship, Start Game spawns a fresh match at 0-0, human
      is Blue slot 0, bots are 4 Blue + 5 Red fresh assignments.
- [ ] `match_id` strictly increases across Quit → re-enter (regression guard
      for the `MatchState::new()` trap).
- [ ] Play Again mid-session unchanged: scores reset, teams persist,
      countdown runs, all respawn at team base. No title-screen flash
      regression (this plan doesn't touch `handle_play_again` — see Notes).
- [ ] Free Roam unaffected: Quit to Title from Free Roam is a no-op
      beyond flipping `game_mode` (which it already is).

## Test plan

**Unit** — `server/src/match_state.rs`:

```rust
#[test]
fn reset_to_waiting_bumps_match_id_and_zeros_scores() {
    let mut s = playing_state();
    s.blue_score = 300;
    s.red_score = 120;
    s.tick(MATCH_DURATION, std::iter::empty());
    let prior = s.match_id;
    s.reset_to_waiting();
    assert_eq!(s.phase, MatchPhase::Waiting);
    assert_eq!(s.match_id, prior.wrapping_add(1));
    assert_eq!(s.blue_score, 0);
    assert_eq!(s.red_score, 0);
}
```

**Manual iPad smoke test** — the acceptance bullets above, run on prod
after deploy.

No integration test. mk48's `ArenaService` has no idiomatic Rust test
harness, so pretending there will be one just produces a skipped checkbox.

## Notes

- **Play Again's title-screen flash is out of scope.** `handle_play_again`
  sets the human's `status = Spawning` on `server/src/server.rs:95`, which
  briefly renders the title screen during the countdown. It's a separate
  defect and doesn't contribute to the stuck-state repro. Track as a
  followup; do not bundle.
- **No `assign_late_joiners` change.** It behaves correctly on clean data.
  The apparent bug was "it picks slot 4 for the human" — that only happens
  because bot teams weren't cleared, which the server teardown above fixes.
- **Risk — iPad `MatchEndOverlay` touch behavior.** The z-index 9999
  backdrop is what blocks Start Game taps in the stuck state. After this
  fix, `match_update.phase` leaves `Ended` at Quit-to-Title time, so
  `MatchEndOverlay` unmounts cleanly. Verify on iPad Safari that a rapid
  Quit-to-Title → re-enter CTA doesn't leave the overlay stuck mid-unmount.

## Files

| File | Change |
|------|--------|
| `server/src/server.rs` | Rewrite `handle_quit_to_title` (~30 LOC) |
| `server/src/match_state.rs` | New `reset_to_waiting` method (~12 LOC) + unit test (~12 LOC) |
| `client/src/ui/game_ui.rs` | New `use_effect_with` in `mk48_ui` (~10 LOC) |

Total: ~65 LOC across three files. One PR, one commit.

## References

- `server/src/server.rs:119` — `handle_quit_to_title` (the bug)
- `server/src/server.rs:87` — `DeathReason::Unknown` precedent
- `server/src/match_state.rs:152, 314` — `new()` and `reset()` to pair with
- `client/src/ui/game_ui.rs:63-74` — persistent `use_state` cells
- `db75e18` — the prior fix that made `MatchPhase::Ended` terminal in the
  tick loop. This plan extends that invariant to the Quit-to-Title path.
- `docs/solutions/logic-errors/terminal-state-overwritten-by-auto-advance.md`
  — the compound doc for the ancestor pattern.
