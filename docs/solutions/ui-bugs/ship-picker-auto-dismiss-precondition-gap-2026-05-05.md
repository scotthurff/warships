---
title: "Ship picker auto-dismissed by retry timer with too-broad precondition (regression of cta-spawn-stuck-ship-picker fix)"
date: 2026-05-05
category: ui-bugs
module: client/src/ui/game_ui.rs
problem_type: ui_bug
component: frontend_stimulus
symptoms:
  - "Ship-picker overlay disappeared ~2 s after appearing, before the player could choose a ship"
  - "Player auto-spawned as the fallback G5 destroyer without tapping Start Game"
  - "Status flipped Spawning → Playing while the picker was still mounted"
  - "Affected both Free Roam and Capture the Area modes (both flow through ShipSelect)"
root_cause: missing_validation
resolution_type: code_fix
severity: high
related_components:
  - "ShipSelect title-flow step"
  - "Spawn retry interval"
  - "UiEvent::Spawn dispatch"
tags:
  - yew
  - rust-wasm
  - use-effect
  - gloo-timers
  - retry-loop
  - precondition-gap
  - ship-picker
  - spawn-flow
  - regression
---

# Ship picker auto-dismissed by retry timer with too-broad precondition

## Problem

The ship-picker overlay auto-dismissed roughly 2 seconds after appearing in Free Roam and Capture the Area, dropping the player into the game piloting a fallback G5 destroyer before they could tap a ship or hit Start Game. Affected every player whose reaction time exceeds 2 s — i.e., everyone.

This is a **regression from the earlier fix** at [`cta-spawn-stuck-ship-picker.md`](cta-spawn-stuck-ship-picker.md) (commit `ba971bc`). That fix added a `gloo_timers::callback::Interval` to recover from server "no spawn slot" failures by re-emitting Spawn every 2 s. The retry's gate was too broad and produced this auto-dismiss bug.

## Symptoms

- Player taps a game mode, the ship picker renders correctly (Back, level nav, ship grid, detail panel, "Start Game" button).
- ~2 s later, with no input, the picker disappears and the player is in-game.
- Player is always piloting a G5 destroyer regardless of what they intended to pick.
- Reproduces in both Free Roam and CTA, every time, on a fresh session.

## What Didn't Work

The retry block (`client/src/ui/game_ui.rs:171-189` pre-fix) had a comment that explicitly named the recovery scenario it was built for: *"if the player clicks Start Game but the server can't find a spawn slot..."*. The actual gate, however, was `spawning && at_picker` — both true the moment the picker first mounts, before any tap. The comment documented a precondition the code never enforced.

Existing UI state-machine tests cover *"when start fires, spawn dispatches"* but nothing asserted the negative — *"when start has NOT fired, spawn must NOT dispatch within N seconds."* Because the bug only manifests after wall-clock 2 s, no synchronous test surfaced it. Visual code review missed it because the retry block reads sensibly in isolation; the gap is between the comment and the gate, not in either alone.

The original session that landed the fix [`ba971bc`](cta-spawn-stuck-ship-picker.md) (session history) noted three server-side spawn-failure modes (Essex too large for spawn ring, pentagon slot on terrain, human slot occupied by bot) and chose client-side retry over a `SpawnFailed` wire message. That choice was correct, but the retry's gate was implemented from the symptom ("we need to retry while picker is up") rather than from the precondition ("only after user committed").

## Solution

Added a `start_requested` state cell flipped on the Start Game tap, plumbed it into the retry's deps, and added a reset effect tied to leaving the picker step.

Before (`client/src/ui/game_ui.rs:171-189`):

```rust
let spawning = matches!(status, UiStatus::Spawning);
let at_picker = *title_step == TitleStep::ShipSelect;
use_effect_with((spawning, at_picker), move |(spawning, at_picker)| {
    if !(*spawning && *at_picker) {
        return Box::new(|| ()) as Box<dyn FnOnce()>;
    }
    let interval = gloo_timers::callback::Interval::new(2000, move || {
        play_cb.emit(default_alias());
    });
    Box::new(move || drop(interval)) as Box<dyn FnOnce()>
});
```

After (`client/src/ui/game_ui.rs:153-217`):

```rust
// True only after the player taps Start Game.
let start_requested = use_state(|| false);

let on_start_from_picker = {
    let play_cb = on_play.clone();
    let start_requested = start_requested.clone();
    Callback::from(move |_: MouseEvent| {
        start_requested.set(true);
        play_cb.emit(default_alias());
    })
};

// Reset start_requested whenever the player leaves the picker.
{
    let start_requested = start_requested.clone();
    let at_picker = *title_step == TitleStep::ShipSelect;
    use_effect_with(at_picker, move |at_picker| {
        if !*at_picker {
            start_requested.set(false);
        }
        || ()
    });
}

// Spawn-retry, now gated on (spawning, at_picker, started).
{
    let play_cb = on_play.clone();
    let spawning = matches!(status, UiStatus::Spawning);
    let at_picker = *title_step == TitleStep::ShipSelect;
    let started = *start_requested;
    use_effect_with(
        (spawning, at_picker, started),
        move |(spawning, at_picker, started)| {
            if !(*spawning && *at_picker && *started) {
                return Box::new(|| ()) as Box<dyn FnOnce()>;
            }
            let interval = gloo_timers::callback::Interval::new(2000, move || {
                play_cb.emit(default_alias());
            });
            Box::new(move || drop(interval)) as Box<dyn FnOnce()>
        },
    );
}
```

## Why This Works

The new gate enforces the precondition the comment already claimed: the retry only runs *after* an explicit Start Game tap. `spawning && at_picker` is now necessary but not sufficient — the third bit, `started`, is the discriminator between "user just landed here" and "user just asked to spawn." The reset effect ties `start_requested`'s lifecycle to the picker step itself, so going Back, switching modes, or completing a spawn (status leaves Spawning → title flow unmounts) all clear the flag. A stale `started=true` cannot leak into the next session.

The original recovery behavior is preserved: if the player taps Start Game and the server's spawn fails (any of the three failure modes documented in [`cta-spawn-stuck-ship-picker.md`](cta-spawn-stuck-ship-picker.md)), the retry still fires every 2 s until status leaves Spawning. The fix is a tightening of the gate, not a removal of the mechanism.

## Prevention

**The "silent precondition gap" rule.** When a `use_effect_with` body has side effects (timers, dispatches, network) AND its surrounding comment names a precondition (e.g., "only fires after X"), the gate expression must reference that precondition by name. A bare `(spawning, at_picker)` tuple gating a retry whose comment says "after Start Game" is a review-blocker. Diff the comment's stated preconditions against the gate's actual operands every time.

**A retry interval gated on UI presence (`screen_X_visible`) must also be gated on user intent (`user_committed_action`).** Mounting a screen is not the same as the user choosing to act on it. The deps tuple should encode both.

**Default new `Interval`-based effects to "off"** with an explicit `armed: bool` flag rather than relying on incidental state coincidence to mean "user took the action."

**Test pattern for recovery-only timers — assert the negative timing path:**

```rust
// Mount the title flow, advance to ShipSelect, advance virtual time
// past the retry period without firing on_start_from_picker, assert
// zero UiEvent::Spawn dispatches.
#[wasm_bindgen_test]
async fn ship_picker_does_not_auto_dismiss() {
    let dispatched: Rc<RefCell<Vec<UiEvent>>> = Default::default();
    // ... mount Mk48Ui with a recording dispatch sink ...
    // advance title_step to ShipSelect
    // gloo_timers::future::sleep(Duration::from_millis(2500)).await;
    assert!(dispatched.borrow().iter().all(|e| !matches!(e, UiEvent::Spawn { .. })));
}
```

Mirror this for every recovery-style timer (any `Interval` whose comment starts with "if the user...").

**Cross-stratum check.** Grep for other `use_effect_with(... gloo_timers ...)` sites to confirm none have the same "visibility-only" gate. The session that landed `ba971bc` cited `cta_respawn_overlay.rs:12,37,40` as prior art for the pattern; verify those gate on user intent or are genuinely fire-on-visible by design. Same family as ["never collapse semantically distinct states into one guard"](../logic-errors/terminal-state-overwritten-by-auto-advance.md).

## Related Issues

- [`cta-spawn-stuck-ship-picker.md`](cta-spawn-stuck-ship-picker.md) — the **direct ancestor**. Documents the original "picker won't dismiss" bug and the retry-interval fix that this doc is a follow-up to. The retry mechanism is unchanged; only its gate is tightened.
- [`../logic-errors/terminal-state-partial-teardown.md`](../logic-errors/terminal-state-partial-teardown.md) — same family. Articulates "Yew `use_state` cells in long-lived components persist across `UiStatus` transitions; tie them to a server-authoritative signal and reset on change." The reset-on-leaving-picker effect added here is that template applied to a sub-step boundary.
- [`../logic-errors/terminal-state-overwritten-by-auto-advance.md`](../logic-errors/terminal-state-overwritten-by-auto-advance.md) — adjacent. The shape "a guard fires when it shouldn't, because the predicate collapses two semantically different states into one branch" matches.
