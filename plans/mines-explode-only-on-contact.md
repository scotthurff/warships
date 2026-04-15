# fix: Mines only explode on direct ship contact

> **Status:** Planned — 2026-04-15
> **Scope:** ~5 LOC, one file, one PR
> **Motivation:** Kid-friendly game — mines shouldn't hunt players.

## Problem

Today mines actively **gravitate toward boats within 60 m proximity**.
Once a ship passes close enough, the mine accelerates toward it and
detonates on impact. Effectively a 60-m kill radius with guided
homing — not a mine, a short-range anti-ship missile.

For a game aimed at ages 5–10 this reads unfair: the kid sees a mine
nearby, thinks they've steered clear, and dies anyway because the mine
curved toward them. The desired behavior is the classic passive naval
mine — drifts with the current, explodes *only* when a ship actually
sails into it.

## Root behavior

`server/src/world_physics_radius.rs:172-175` inside the pairwise
boat×weapon interaction block:

```rust
// Mines also gravitate towards boats (even submerged subs).
if boats.len() == 1 && weapons.len() == 1
    && weapons[0].data().sub_kind == EntitySubKind::Mine
    && weapons[0].is_in_proximity_to(boats[0], Entity::CLOSE_PROXIMITY)
{
    let weapon_position = weapons[0].transform.position;
    let closest_point = boats[0].closest_point_on_keel_to(weapon_position, 1.0);
    mutate(
        weapons[0],
        Mutation::Attraction(
            closest_point - weapon_position,
            Velocity::from_mps(MINE_SPEED),
            boats[0].altitude - weapons[0].altitude,
        ),
    );
}
```

`Entity::CLOSE_PROXIMITY = 60.0` at `server/src/entity.rs:635`.

There is **no separate proximity-fuse detonation** — mines don't
explode from being near a ship; they explode from the resulting
SAT collision after they've been pulled into contact. So removing the
gravitation is the whole fix.

## Fix

Delete the `if boats.len() == 1 && … Mine …` block entirely. Mines
keep their current passive drift (via `Mutation::*` from elsewhere in
physics) and their current lifespan expiry (`Fate::Remove(Unknown)` at
`world_physics.rs:67` — silent disappearance, no damage, which is
correct for a mine that was never triggered). Collision handling is
unchanged: when a boat does hit a mine, the existing weapon-on-boat
collision path detonates it.

## Acceptance

- [ ] A boat sailing past a mine at 10-50 m clearance does **not**
      trigger the mine.
- [ ] A boat sailing directly into a mine **does** trigger it (normal
      collision damage).
- [ ] Submerged submarines are no longer auto-targeted by mines at
      range.
- [ ] Mines still despawn silently when their lifespan expires.
- [ ] No regression to depth-charge proximity detonation
      (`EntityData::DEPTH_CHARGE_PROXIMITY`, separate code path at
      `world_physics_radius.rs:270-278`).

## Test plan

**Manual smoke test** on prod:

1. Spawn as any mine-carrying ship (e.g., a Destroyer with Mine
   armament; check `EntitySubKind::Minelayer` loadouts).
2. Drop a mine behind you. Circle back and pass 30 m away — expect
   no detonation.
3. Sail directly into the mine — expect damage/death.
4. Drop another mine near a submerged submarine — mine should NOT
   move toward the sub.
5. Drop a mine and leave it; wait for lifespan (~300 ticks) to expire
   — mine silently despawns with no visible explosion.

No unit tests needed — the change is a deletion of ~4 lines in a
physics pairwise handler, exercised by any manual playthrough.

## Files

| File | Change | LOC |
|------|--------|-----|
| `server/src/world_physics_radius.rs` | Delete the mine-gravitation block | −5 |

## Notes

- **Mine speed parameter** (`MINE_SPEED` constant referenced in the
  deleted block) becomes unused. Grep for other references; if none,
  also delete the const. If it's used by mine-drift physics elsewhere,
  leave it.
- **Kodiak / upstream mk48 behavior**: this change diverges from
  upstream, which assumes a faster-paced adult game where guided
  mines add tactical depth. Warships is explicitly not that game
  (CLAUDE.md: "kid-friendly single-player naval combat game for
  iPad"). Document in the commit.

## References

- `server/src/world_physics_radius.rs:170-176` — the gravitation block
- `server/src/entity.rs:635` — `CLOSE_PROXIMITY = 60.0`
- `common/src/entity/_type.rs` — mine entity props (search for
  `sub_kind = "mine"` or `EntitySubKind::Mine`)
- CLAUDE.md — kid-friendly target audience
