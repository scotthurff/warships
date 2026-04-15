# fix: CTA bots crash into land in own territory — carve the arena flat

> **Status:** Planned — 2026-04-15
> **Scope:** One server-side change, ~30 LOC, one PR
> **Motivation:** Playtest: "bots always crash into land within their own territory in 5v5"
> **Review:** DHH / Kieran / Simplicity (2026-04-15, unanimous: eliminate the bug class, don't tune around it)

## Problem

Live playtest: teammate bots in Capture the Area consistently beach
themselves on terrain inside their own half of the arena. This
reads as "my ally crashed into a rock" — a trust-breaker for a
kid-friendly game.

The 1500-radius CTA arena (`server/src/match_state.rs:44-47`)
overlaps the `ARCTIC` biome line at y = 1250 (`common/src/world.rs:12`).
Random procedural terrain leaves land features scattered inside the
arena. The bot AI's local terrain repel at `server/src/bot.rs:142-154`
samples 10 directions at `data.length` distance — enough for most
passes in open water, not enough when a 2.5-magnitude CTA objective
pull drags a bot into ice on the northern arc.

## Fix

**Carve the arena flat of terrain on match start.** One call in the
existing CTA bootstrap path alongside `clear_statics()`. No per-tick
cost. No tuning constants. Eliminates the failure class.

```rust
// server/src/server.rs, new method on Server
const CARVE_DEPTH: f32 = -25.0;  // low enough to be solid open water
const CARVE_STEP: f32 = 40.0;    // terrain chunk scale — adjust if
                                  // short-range holes appear between samples

fn flatten_cta_arena(&mut self) {
    let r = ArenaLayout::DEFAULT.arena_radius;
    let mut y = -r;
    while y <= r {
        let half_width = (r.powi(2) - y.powi(2)).sqrt();
        let mut x = -half_width;
        while x <= half_width {
            self.world.terrain.modify(TerrainMutation::simple(
                Vec2::new(x, y),
                CARVE_DEPTH,
            ));
            x += CARVE_STEP;
        }
        y += CARVE_STEP;
    }
}
```

Called once from the existing `start_match` branch at
`server/src/server.rs:793-803`:

```rust
if matches!(self.match_state.phase, MatchPhase::Waiting) {
    self.match_state.start_match();
    self.assign_match_teams();
    self.clear_statics();
    self.flatten_cta_arena();   // ← new
    info!(...);
}
```

Same call from the Spawn-handler path at `server/src/server.rs:537-544`
for the first-spawn bootstrap.

## Why this over "tune the bot repel"

All three plan reviewers flipped the original plan's
tune-primary / carve-fallback framing. The short version:

- Carving eliminates the failure class. Tuning reduces its
  frequency against a moving target — any future bump to the
  objective-pull weight re-opens the bug.
- Carving is one-shot O(r² / step²) ≈ 5,600 terrain calls per
  match start. No per-tick cost. Bot sampling adds 3,240 terrain
  lookups *per second* forever.
- Tuning would leave five magic numbers in `bot.rs`
  (sample ring multipliers, repel denominator, speed scaling) that
  nobody will remember the provenance of in six months.

This plan does **not** touch `bot.rs`. Bot AI tuning for
offense/defense awareness is tracked in a separate plan
(`plans/cta-bot-targeting.md`), which ships after this one has
been observed for at least one match.

## Acceptance

- [ ] Over 5 consecutive CTA matches, **zero** teammate bot deaths
      with `DeathReason::Terrain`. Instrument via a per-match
      counter logged at match end (dev-only).
- [ ] A freshly started CTA match has walkable water across the
      entire 1500-radius arena — no visible ice/rock formations
      inside the circle on the client-side terrain render.
- [ ] Free Roam is unaffected — its terrain generation path stays
      random.
- [ ] Match start time is not visibly delayed. The carve runs
      inside the existing `start_match` transition, which already
      does `clear_statics()` (an O(n²) static-entity scan).

## Test plan

**Dev instrumentation** (optional, keep behind `cfg!(debug_assertions)`):
Add a counter `bot_terrain_deaths_this_match: u32` on `Server` that
increments when a bot boat dies with `DeathReason::Terrain`. Log at
`MatchEvent::MatchEnded`. Delete after first clean playtest — the
instrument is a one-shot validator, not permanent telemetry.

**Manual smoke test** on prod, iPad + desktop:

1. Hard-refresh. Pick CTA → any ship → Start Game.
2. At countdown, observe: minimap shows open water across the arena.
   No brown/white terrain features inside the 1500-radius circle.
3. Play through a full 5-minute match. Watch for teammate bots
   beaching on the arctic arc or southern edge. Expect zero.
4. Repeat on Captain and Fleet Commander difficulties — no
   difficulty-specific regressions.
5. Hit Play Again. Second match starts on a freshly-carved arena
   (the `start_match → flatten_cta_arena` path fires again via the
   `reset() → Countdown` transition re-entering `start_match`? —
   verify. If not, call `flatten_cta_arena` from `reset_to_waiting`
   too, or from the Play Again handler at `server.rs:59-111`).

## Risks

| Risk | Mitigation |
|------|------------|
| `TerrainMutation::simple(pos, -25.0)` is additive not absolute — a chunk with altitude +40 becomes +15, not -25. May not fully flatten steep peaks. | Test with deepest observed peak on a freshly-generated arena. If insufficient, iterate with larger negative delta or a loop that re-applies until `terrain.sample(pos) < SAND_LEVEL`. |
| The 40-unit step grid leaves gaps where a chunk-internal bump is between samples. | Shorten step (20 units) if gaps show. Each halving quadruples calls but they run once per match. |
| Play Again entry doesn't re-call the carve (if `match_state.reset()` → Countdown bypasses the Waiting-phase gate). | Verify and patch — carve must run on every match start, not just the first. Add to `handle_play_again` explicitly if needed. |
| Carving alters `Mutation::conditional` behavior used elsewhere. | `TerrainMutation::simple` is already used at `server/src/world_inbound.rs:512` and `world_physics.rs:350` — well-trodden API. Low risk. |

## Files

| File | Change | Rough LOC |
|------|--------|-----------|
| `server/src/server.rs` | Add `flatten_cta_arena`; call from `start_match` branch and possibly from `handle_play_again` | ~30 |

One commit, one PR.

## References

- `server/src/match_state.rs:30-48` — `ArenaLayout::DEFAULT`
  (`arena_radius: 1500`)
- `common/src/world.rs:12` — `ARCTIC = 1250` (the biome line the
  arena overlaps)
- `common/src/terrain.rs:281, 659` — `TerrainMutation` and
  `terrain.modify`
- `server/src/world_inbound.rs:512`,
  `server/src/world_physics.rs:350` — existing
  `TerrainMutation::simple` callsites to mirror
- `server/src/server.rs:147-174` — `clear_statics` pattern to
  model after (same "on match start" lifecycle hook)
- Prior compound doc: `docs/solutions/logic-errors/terminal-state-partial-teardown.md` —
  relevant for the "run on every match start path, not just
  Waiting→Countdown" reminder
