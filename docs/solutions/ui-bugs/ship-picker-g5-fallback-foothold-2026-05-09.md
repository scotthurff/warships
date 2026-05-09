---
title: "Ship picker auto-dismisses again — G5 fallback in `on_play` was the real foothold"
date: 2026-05-09
category: ui-bugs
module: client/src/ui/game_ui.rs
problem_type: defense_in_depth_gap
component: frontend_stimulus
symptoms:
  - "Ship-picker overlay disappears ~2 s after the player taps Continue, before they can select a ship"
  - "Player auto-spawns as a G5 destroyer with no ship input"
  - "Reproduces even with the (spawning, at_picker, started) gate from aefe578 supposedly in place"
root_cause: silent_default_value_emit
resolution_type: code_fix
severity: high
related_components:
  - "Spawn-retry Interval"
  - "on_play closure"
  - "Start Game button"
tags:
  - yew
  - rust-wasm
  - use-effect
  - gloo-timers
  - default-value-foothold
  - ship-picker
  - spawn-flow
  - regression
  - defense-in-depth
---

# G5 fallback in `on_play` was the foothold

## Problem

The ship picker auto-dismissed ~2 s after the player tapped **Continue**, dropping
them into the game piloting a G5 destroyer with no ship input. This is the
same surface symptom as the original 2026-05-05 bug
([`ship-picker-auto-dismiss-precondition-gap-2026-05-05.md`](ship-picker-auto-dismiss-precondition-gap-2026-05-05.md)),
which had been "fixed" in commit `aefe578` by adding a `start_requested`
flag to the retry interval's deps tuple. The bug recurred.

## Why the previous fix was incomplete

The 2026-05-05 fix gated the retry interval on
`(spawning, at_picker, start_requested)`. Its correctness argument was: the
retry can only fire if the user explicitly tapped **Start Game**. That argument
is sound *for that one path*. It treats the retry interval as the only way a
spurious spawn can leak through.

It didn't ask the inverse question: *if* a spawn does fire spuriously — by any
mechanism we did or didn't enumerate — what does it do? The answer was
hiding in `on_play`:

```rust
let on_play = {
    let mode = *selected_mode;
    let selected_ship = selected_ship.clone();
    gctw.send_ui_event_callback.reform(move |alias| UiEvent::Spawn {
        alias,
        entity_type: selected_ship.unwrap_or(EntityType::G5),  // ← the foothold
        game_mode: mode,
    })
};
```

Any path that fires `on_play.emit()` while `selected_ship` is `None` quietly
spawns the player as a G5. The retry was one such path; the *reason* the bug
was so reproducible is that the fallback turned every spurious emit into a
working spawn, regardless of whether the gate logic was right.

The shape of the bug: a single carefully-reasoned gate stood between
"spurious emit" and "phantom spawn." The default value silently completed
the spawn even if the gate was wrong, and "is the gate right?" became
unverifiable in isolation because the failure mode was indistinguishable
from "user clicked Start Game with no ship picked, somehow."

## Symptoms

- Tap a game mode tile → tap **Continue** → ship picker renders correctly.
- ~2 s later, with no further input, the picker disappears and the player is
  in-game as a G5.
- Reproduces fresh from page load. Reproduces in both Free Roam and CTA.
- The picker visually appears for the full 2 s — there's no flash; the
  auto-dismiss is the spawn completing.

## What Didn't Work

The 2026-05-05 fix landed `start_requested` and a reset effect tied to
`at_picker`. Code review verified the gate's logic. But the gate itself
became a single point of correctness: any path the maintainer hadn't
enumerated (kodiak global event handling, iOS Safari synthesized clicks
through a "disabled" attribute, a Yew effect-ordering quirk we don't have
test coverage for) that managed to flip `start_requested` true *or* call
`on_play.emit()` directly would route through the G5 fallback and produce
the exact symptom — even though the documented fix appeared correct.

Asserting "the gate is right" doesn't equal "spurious spawns are
impossible." The fallback was the difference.

## Solution

Two structural changes plus one belt-and-suspenders gate:

### 1. Remove the G5 fallback from `on_play`

```rust
// Before — Callback<PlayerAlias> via reform
gctw.send_ui_event_callback.reform(move |alias| UiEvent::Spawn {
    alias,
    entity_type: selected_ship.unwrap_or(EntityType::G5),
    game_mode: mode,
})

// After — explicit Callback that no-ops without a pick
let send_ui_event = gctw.send_ui_event_callback.clone();
Callback::from(move |alias: PlayerAlias| {
    if let Some(entity_type) = *selected_ship {
        send_ui_event.emit(UiEvent::Spawn { alias, entity_type, game_mode: mode });
    }
})
```

This is the fix. After this change, no spawn dispatches without an explicit
ship pick — full stop, by construction, not by gating. Whether the retry
gate is right or wrong, whether `start_requested` leaks across some session
boundary we haven't enumerated, whether iOS Safari synthesizes a phantom
click on the disabled button — none of those produce a phantom spawn,
because there's no longer a default ship to spawn into.

### 2. Make `on_start_from_picker` idempotent without a pick

```rust
Callback::from(move |_: MouseEvent| {
    if selected_ship.is_some() {
        start_requested.set(true);
        play_cb.emit(default_alias());
    }
})
```

The button is `disabled={!has_selection}` already. This is the
JavaScript-side mirror of that, in case some platform path bypasses the
HTML disabled attribute (iOS Safari synthesized clicks have done this for
us before — see `ios-safari-touch-action-and-tap-highlight.md`).

### 3. Add `selected_ship.is_some()` to the retry-interval deps

```rust
let has_ship = selected_ship.is_some();
use_effect_with(
    (spawning, at_picker, started, has_ship),
    move |(spawning, at_picker, started, has_ship)| {
        if !(*spawning && *at_picker && *started && *has_ship) {
            return Box::new(|| ()) as Box<dyn FnOnce()>;
        }
        ...
```

Redundant given (1) — the retry's emit is now a no-op anyway — but it
prevents the timer from even *arming* in the no-ship case, which is what
the comment on the effect now documents as the contract. Two independent
gates protecting the same invariant beats one carefully-reasoned gate.

### 4. Reset `start_requested` in the in_match effect

```rust
use_effect_with(in_match, move |currently_in_match| {
    if !*currently_in_match {
        ...
        start_requested.set(false);  // ← added
    }
    || ()
});
```

Belt-and-suspenders alongside the existing `at_picker` reset. Closes a
transient window during Quit-to-Title where status flips
Playing→Spawning before `title_step` propagates back to `ModeSelect`,
briefly satisfying `(spawning, at_picker, started)` before the
`at_picker` effect catches up.

## Why This Works

**Defense in depth, not defense in detail.** The previous fix was
"defense in detail" — one carefully-reasoned gate doing all the work,
relying on the maintainer to enumerate every path that could spawn the
player. This fix moves the invariant *into the dispatcher itself*: a
spawn requires `Some(entity_type)` to be constructed at all. The retry
gate, the button's disabled attribute, the start_requested flag — all
remain as additional layers, but the dispatcher is the structural
guarantee. Any future path we haven't thought of that calls
`on_play.emit()` is harmless.

The original recovery behavior is preserved: if the user picks a ship,
taps Start Game, and the server fails to find a spawn slot, the retry
still fires every 2 s with the picked ship until the server succeeds.

## Prevention

**The "default-value foothold" rule.** When a callback dispatches a
side-effecting event and one of its inputs has a default that produces
a *valid* event (vs. a panic, an error, a no-op), that default is a
foothold. Any spurious dispatch route routes through the default and
produces a working — wrong — outcome. Audit your dispatchers:
*"Is there an input here whose default would produce a working but
unintended action?"* If yes, replace the default with a no-op or
explicit error.

**Defense in depth for irreversible actions.** A spawn, a payment, a
deletion — anything the user can't trivially undo — should be guarded by
at least two independent layers, each sufficient on its own:
1. A *structural* layer (the dispatcher itself rejects the action
   without a real input).
2. A *gating* layer (the call site only reaches the dispatcher when
   the user has expressed intent).

Either layer alone failing should still block the action.

**When a fix is "logically correct," ask: what would the failure mode
look like if the logic was wrong?** If the answer is "exactly this
bug," the fix is too dependent on the logic being right. Add a
redundant guard that doesn't share the same reasoning.

**Specifically for spawn flows:** treat `selected_ship.is_some()` as
the precondition for *any* spawn dispatch, not just user-initiated
ones. The default in `unwrap_or(...)` was load-bearing — when a
default is load-bearing for correctness, that's a smell.

## Cross-stratum check

Other places in this codebase that use `unwrap_or` in event-emit
paths should get the same audit:

```bash
grep -rn "unwrap_or\|or_default\|or_else" client/src --include="*.rs" \
  | grep -i "emit\|send\|dispatch"
```

Each result: does the default produce a working but unintended action?
If yes, replace with explicit handling.

## Related Issues

- [`ship-picker-auto-dismiss-precondition-gap-2026-05-05.md`](ship-picker-auto-dismiss-precondition-gap-2026-05-05.md)
  — the **direct ancestor**. That fix added `start_requested` to the
  retry's deps. This fix removes the G5 fallback that made the retry's
  spurious-emit failure mode produce a working spawn. The two fixes
  compose; neither replaces the other.
- [`cta-spawn-stuck-ship-picker.md`](cta-spawn-stuck-ship-picker.md)
  — the **grandparent**. Documents why the retry interval exists in
  the first place (server "no spawn slot" recovery). The retry is
  preserved; only its protective layers are tightened.
- [`ios-safari-touch-action-and-tap-highlight.md`](ios-safari-touch-action-and-tap-highlight.md)
  — context for "iOS Safari can synthesize clicks in surprising
  places." Part of the reason we want the dispatcher itself to refuse
  spawns without a pick.
