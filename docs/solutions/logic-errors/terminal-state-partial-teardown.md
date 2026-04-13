---
title: Terminal state left dirty by partial teardown on user-exit path
category: logic-errors
component: server/match-state
problem_type: state_machine_bug
symptoms:
  - "Ship picker soft-locks — Start Game not tappable"
  - "Minimap shows stale bot ships from the previous match"
  - "Only reproduces after N matches and a Quit-to-Title (not on first run)"
  - "MatchEndOverlay (z-index 9999) swallows taps meant for the underlying UI"
severity: high
resolution: fixed
fixed_in: d8f7320
related_files:
  - server/src/server.rs
  - server/src/match_state.rs
  - client/src/ui/game_ui.rs
tags: [state-machine, fsm, teardown, rust, yew, idempotence]
related_docs:
  - ../logic-errors/terminal-state-overwritten-by-auto-advance.md
---

# Terminal state left dirty by partial teardown on user-exit path

## Symptom

Reproducible after playing multiple Capture-the-Area matches in a row:

1. Play a 5v5 CTA match end to end.
2. Click **Play Again** two or three times.
3. On the final match-end screen, click **Quit to Title**.
4. On the title screen pick **Capture the Area → Continue**.
5. Ship picker renders, but **Start Game is untappable** and the
   **minimap is already showing bot ships** from the prior match.

The user cannot advance out of this state. Hard-refresh unblocks.

## Root cause

One bug, three loud symptoms. The bug: **`handle_quit_to_title` on
`server/src/server.rs` was a partial teardown.** It only mutated the
quitting human's fields. Everything else — the FSM, the bot roster,
client-side picker state — stayed dirty.

Specifically:
- `match_state.phase` stayed at `MatchPhase::Ended { winner }`. No code
  path existed to transition it out except `handle_play_again`.
- Bot players kept `match_team = Some(Blue/Red)`, `match_slot`,
  `selected_loadout` from the prior match. Their boats stayed alive
  and floating in the arena.
- The client's `mk48_ui` kept its `title_step = ShipSelect` and
  `selected_ship = Some(IowaOrWhatever)` from the prior session.

The symptoms fell out like this:

- **Start Game untappable**: the client kept receiving `MatchUpdate`
  with `phase = Ended`, which renders `MatchEndOverlay` at `z-index: 9999`
  covering the full viewport. Taps hit the overlay, not the ship picker
  button beneath.
- **Stale bot ships on minimap**: bot boats were never despawned, and
  the server's `get_game_update` kept surfacing their positions in the
  `MatchUpdate.players` array.
- **"Only after N matches"**: the first few Play-Again cycles "worked"
  (each Play Again briefly bounces the human to the title screen during
  the new countdown, but because `selected_ship` persists across renders,
  the user can re-click Start Game and spawn again — they just don't
  notice the flicker). State accumulates silently. Quit-to-Title is the
  first action that requires the server to actually clean up, and it
  didn't.

## Why the previous fix wasn't enough

Ancestor doc: `terminal-state-overwritten-by-auto-advance.md`. That fix
made `MatchPhase::Ended` genuinely terminal in the tick loop — the tick
loop stopped auto-restarting matches from `Ended` every 100ms, so the
`MatchEndOverlay` got enough frames to render.

That fix enforced one-half of the terminal-state invariant: **nothing
advances out of `Ended` automatically**. It did not enforce the other
half: **a human-gated exit must actually reset**. Play Again did
(`match_state.reset()`). Quit-to-Title didn't.

Pattern worth remembering: **every path out of a terminal state needs
to be audited separately.** Making the state terminal on the "keep
playing" side doesn't help if the "leave" side is a no-op.

## Fix

Three landed changes in one commit (`d8f7320`).

### 1. Named teardown method on the FSM

`server/src/match_state.rs` — new `reset_to_waiting()` sibling to the
existing `reset()`:

```rust
/// Tear down the current match and return to Waiting. Preserves
/// match_id monotonicity so clients discard stale MatchUpdates.
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

Critical: **not `MatchState::new()`**. `new()` hard-codes `match_id: 1`,
which would collide with in-flight client packets holding `match_id: 7`
or whatever the prior session bumped to. The `wrapping_add(1)` preserves
the discard-stale-packets invariant the client relies on.

`reset()` (Play Again → Countdown) and `reset_to_waiting()` (Quit to
Title → Waiting) now form a symmetric pair. The Waiting-vs-Countdown
distinction is encoded in the method name, not a comment.

### 2. Full teardown in `handle_quit_to_title`

Five numbered steps, no conditional mutation of the quitting player in
isolation:

```rust
fn handle_quit_to_title(&mut self, player_id: PlayerId) {
    info!("match {} quit-to-title …", self.match_state.match_id);

    // 1. Collect every team-assigned boat before mutating.
    let doomed: Vec<EntityIndex> = self.player.iter_borrow()
        .filter_map(|p| {
            if p.match_team.is_some() {
                if let Status::Alive { entity_index, .. } = p.status {
                    return Some(entity_index);
                }
            }
            None
        })
        .collect();

    // 2. Remove boats from the world. DeathReason::Unknown matches
    //    handle_play_again's precedent — Debug panics in on_world_remove.
    for idx in doomed {
        self.world.remove(idx, DeathReason::Unknown);
    }

    // 3. Clear per-player match state on every team-assigned player.
    for mut p in self.player.iter_borrow_mut() {
        if p.match_team.is_none() { continue; }
        p.match_team = None;
        p.match_slot = 0;
        p.selected_loadout = None;
        p.match_stats = PlayerMatchStats::default();
        p.status = Status::Spawning;
        p.flags = Flags::default();
    }

    // 4. Flip the quitting human to Free Roam. Bots default to
    //    FreeRoam already, so any_cta_player drops to false.
    if let Some(mut human) = self.player.borrow_player_mut(player_id) {
        human.game_mode = GameMode::FreeRoam;
    }

    // 5. Park match_state at Waiting.
    self.match_state.reset_to_waiting();
}
```

Deleted in the same edit: the buggy `flags.left_game = true; …
= false;` toggle (setting true then immediately false was a no-op — the
prior author clearly intended "flag the boat for cleanup next tick,"
but the second write clobbered the first).

### 3. Client title-state reset tied to match_id

`client/src/ui/game_ui.rs` — `use_effect_with` in `mk48_ui` watching the
server-provided match_id:

```rust
{
    let selected_mode = selected_mode.clone();
    let selected_ship = selected_ship.clone();
    let title_step    = title_step.clone();
    let session_key   = props.match_update.as_ref().map(|m| m.match_id);
    use_effect_with(session_key, move |_| {
        selected_mode.set(GameMode::FreeRoam);
        selected_ship.set(None);
        title_step.set(TitleStep::ModeSelect);
        || ()
    });
}
```

`match_id` changes on exactly two events:
- **Play Again** (`reset()`): user stays in-match, title screen isn't
  rendered, reset is invisible.
- **Quit to Title** (`reset_to_waiting()`): `match_update` drops to
  None for the quitter, effect fires, user lands on `ModeSelect` with
  no pre-selected ship.

Using `match_id` (a direct server signal) rather than inferring from
`match_update.is_some()` gives us an unambiguous dependency that survives
future refactors. This was a specific reviewer catch — watching a
derived boolean would have coupled the client reset to an accidental
characteristic of the protocol.

## Prevention

**Exit paths from a terminal state need to be as thoroughly audited as
entry paths.** If you have two buttons that both "leave" the terminal
state (`Play Again` and `Quit to Title`), they must both do a complete
reset. If one of them is "lighter" (keeps some state the other doesn't),
that's fine — but write the difference out loud in the method bodies.
Don't let one handler silently no-op.

**Name your FSM transitions after their intent, not their construction.**
`MatchState::new()` at a Quit callsite looks like "build a fresh
default" and hides the subtle fact that `match_id: 1` is a regression.
`MatchState::reset_to_waiting()` names the transition. A reader at the
callsite doesn't have to chase the method body to understand what's
happening to the client contract.

**Count symptoms, not bugs.** The initial diagnosis had this as "4
bugs." Three reviewers independently pushed back: it's one bug with
three downstream symptoms. Collapsing the count changed the shape of
the fix (one PR instead of four, one commit instead of phased rollout).
If you're writing a plan and the "N bugs" list is growing, ask whether
any of them disappear when you fix the first.

**Watch out for persistent client state across session boundaries.**
Yew's `use_state` cells in a long-lived component (like the top-level
UI) persist across `UiStatus` transitions. If the user's next session
starts cleanly, those cells are a leak. Tie them to a
server-authoritative signal (we used `match_id`) and reset on change.

## Regression test to watch for

Manual smoke test after any change to `handle_quit_to_title`,
`handle_play_again`, or the `MatchPhase` enum:

1. Hard-refresh the client.
2. Pick Capture the Area → Continue → pick any ship → Start Game.
3. Play until match end, click Play Again, play again.
4. Quit to Title from the final match-end screen.
5. Capture the Area → Continue.
6. Expect: ship picker with **no pre-selected ship**, Start Game
   greyed out, no minimap, no HUD clock.
7. Pick a ship → Start Game. Expect: fresh countdown, 0-0 scoreboard,
   clean match.

If step 6 shows a pre-selected ship *or* a visible minimap — the fix
has regressed.

## What to grep for when a similar bug is suspected

- `matches!(phase, TerminalState | …)` — same as the ancestor doc.
- Any command handler that says "mark for cleanup" but mutates state
  through a flag that's written twice (the `left_game = true; … =
  false;` smell).
- Any `use_state` cell in a long-lived Yew component with no
  `use_effect_with` reset hook.
- Any `FsmStruct::new()` call outside the server boot path — usually
  a missing named teardown method.
