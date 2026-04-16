# feat: Force bot objective commitment (remove combat-mode weight penalty)

> **Status:** Planned — 2026-04-15
> **Scope:** 1 line change in `server/src/bot.rs` + acceptance re-run.
> **Motivation:** Phase 4 shipped with 0/9 bots reaching the enemy
> base across a 5-minute match. Root cause: the bot AI drops its
> objective-pull weight from 6.0 → 1.5 whenever an enemy is in
> sensor range, which is always true at midfield. Bots park at
> enemy-engagement distance (200 m) and orbit the scrum instead of
> pushing through. This is what Scott sees as "ZERO aggression in
> movement."

## Diagnosis (from match-1 logs)

Phase 4 match-end data:

```
terrain_deaths=12 enemy_base_reached=0 throttle_rate=46.7%
                                       (12720/27225 ticks)
                   winner=Red blue=80 red=100
9 unique bots entered contested zone
```

- Terrain deaths dropped 376 → 12 (31× reduction).
- 9/9 bots reached the 300 m contested zone at midfield.
- Red scored 100 (= 20 kills) — bots engaged and fought.
- 0 bots reached within 250 m of the enemy base over 5 minutes.

Bots CAN move and DO fight. They just stop at midfield.

## Root cause

`server/src/bot.rs:304-305`:

```rust
let in_combat = closest_enemy.is_some();
let weight = if in_combat { 1.5 } else { 6.0 };
```

`closest_enemy` is `Some` whenever any enemy contact is in the
bot's sensor range. In CTA with 5 bots per team in a 3000-radius
arena, this is true ~100% of the time once the two teams meet.

With `weight = 1.5`, the objective (flow-field pull toward enemy
base) is applied at `movement += flow_dir * 1.5`. At the same
time, each nearby enemy boat contributes a spring force at
`movement += spring_force * 1.0` toward engagement range
(~200 m). With 3–5 enemies in midfield, the enemy springs
collectively outweigh the objective pull 3–5×.

Result: bots sit at engagement range, orbit enemies, shoot at
them, die, respawn, repeat. Never commit to a push past the
scrum. The ships' actual heading oscillates between "toward flow"
and "stay at engagement range," which reads visually as circling.

## Fix

Delete the combat-mode weight penalty. Always use the full
objective-pull weight:

```rust
// server/src/bot.rs:305
-let weight = if in_combat { 1.5 } else { 6.0 };
+let weight = 6.0;
```

Plus one variable cleanup — `in_combat` is no longer used:

```rust
-let in_combat = closest_enemy.is_some();
-let weight = if in_combat { 1.5 } else { 6.0 };
+// Always push the objective. The prior `in_combat ? 1.5 : 6.0`
+// weighting caused bots to park at enemy-engagement range
+// whenever anyone was visible (always, in CTA), outweighing the
+// flow-field pull 3–5× via the summed enemy springs. Bots
+// never made it past midfield. Keep objective dominant; enemy
+// springs at weight 1.0 still draw bots toward targets for
+// shooting, but the flow carries them through.
+let weight = 6.0;
```

`closest_enemy` is still used elsewhere (aiming, firing decisions)
— only the `in_combat` binding and the conditional weight go away.

## What this does NOT do

- Does NOT remove the enemy-engagement spring (bots still orbit
  targets for shooting; just the flow carries them past).
- Does NOT change the difficulty-based speed multipliers or the
  Phase 3b steering throttle.
- Does NOT change how `closest_enemy` is computed or used for
  firing solutions (bot.rs lines further down use it for aim).
- Does NOT touch the defense-mode target swap (bots still pivot
  to own base when defense_progress_ms > 5000).

## Acceptance

Re-run 3 consecutive matches. Gates (adding one new one on top
of the Phase 4 set):

1. **terrain_deaths ≤ 10 per match** (Phase 4 got 12 — should
   stay in this neighborhood; pushing harder may slightly
   increase terrain risk but bots spend less time at midfield
   scrum).
2. **≥ 3 bots per team reach enemy base per match** (Phase 4
   got 0 — the gate this fix is designed to clear).
3. **throttle_rate ≤ 40% match 1, ≤ 25% by match 3** (Phase 4
   got 46.7%; should go DOWN as bots spend less time circling
   in open water and more time on straight runs).
4. **≥ 2 total base captures across the 3 matches** (NEW — was
   the point of the whole CTA mode). Phase 4 had 0 base
   captures (winner decided by kill points). If bots push the
   objective, captures should start happening.

**Hard stop:** if `enemy_base_reached` stays at 0, the problem is
NOT this weight — the theory is wrong and we revert. No further
"tune the weight" iteration.

## Files

| File | Change | LOC |
|------|--------|-----|
| `server/src/bot.rs` | Remove `in_combat` weight conditional | -2 / +7 |
| **Total** | | **~5 LOC net** |

One commit.

## Implementation

1. `server/src/bot.rs:304`: delete `let in_combat = ...`.
2. `server/src/bot.rs:305`: replace `let weight = if in_combat { 1.5 } else { 6.0 };`
   with `let weight = 6.0;` plus the why-comment above.
3. `cargo build && cargo test` — no new tests (trivial change, the
   bot AI has no unit tests today; acceptance is playtest-based).
4. Commit, restart server, play 3 matches.

## References

- `plans/cta-arena-expand-and-sparsen.md` — Phase 4 (shipped 
  terrain fix, surfaced this tactical issue)
- `plans/non-holonomic-ship-steering.md` — Phase 3b (steering
  layer, still in place; orthogonal to this)
- `server/src/bot.rs:304-305` — the weight conditional being
  removed
- `server/src/bot.rs:234-236` — enemy engagement spring (weight
  1.0) — unchanged, still pulls bots toward targets for shooting
- `server/src/bot.rs:259-264` — teammate separation spring
  (weight 0.2) — unchanged, already tuned down to not drown out
  objective
- Match 1 log (this session, unsaved): `steering — terrain_deaths=12
  enemy_base_reached=0 throttle_rate=46.7%`
