# WARSHIPS

A kid-friendly single-player naval combat game for iPad and desktop browser.
Fork of [mk48.io](https://github.com/SoftbearStudios/mk48) with the multiplayer
stripped out, touch controls added, bot AI rewritten for aggressive play, and
a new Capture the Area game mode.

**Live build:** (add URL here once deployed)

---

## What makes this different from mk48.io

- **Single-player with bots** — no online matchmaking, no social features, no
  leaderboards. The game boots straight into a solo session you can play
  offline on your local network.
- **Two modes:**
  - **Free Roam** — the original mk48 sandbox with dynamic arena sizing. You
    and 9 bots roam a large ocean, collect crates, level up, sink each other.
  - **Capture the Area** — a 5-minute 5v5 match mode. Blue (you + 4 bots) vs
    Red (5 bots). Two bases 1000 units apart. Hold the enemy base for 30
    continuous seconds to score 50 points. Kills score 10. Highest score at
    the 5-minute mark wins.
- **Touch controls** — on-screen d-pad, throttle, fire, zoom, and speed
  buttons sized for iPad touch targets (80pt primary, 60pt secondary).
  Controls stay visible once any touch is detected so Safari's synthesized
  mouse events don't flicker them off.
- **Wargame visual design** — Black Ops One for titles, Menlo monospace for
  HUD, translucent dark panels with colored borders. Capture progress rings,
  team-colored ship coloring, minimap, match end results screen.
- **Procedural fleet rotation** — each Capture the Area match picks 5 random
  level-3-to-6 warships (excluding submarines, minelayers, etc.) from the
  ship catalog. Both teams mirror the same fleet for balance. Different
  ships every match.
- **Bot AI overhaul** — bots spring toward enemies at engagement range
  (approach + orbit) instead of fleeing, push the objective when idle, defend
  their own base when it's under attack, and treat CTA teammates as friendly
  for spawn clearance and friendly-fire prevention.

## Tech

| | |
|---|---|
| **Server** | Rust, Actix actors, 10Hz physics tick |
| **Client** | Rust/WASM via Yew, WebGL for world rendering, HTML overlay for HUD |
| **Engine** | [kodiak](https://github.com/scotthurff/warships-kodiak) — vendored as a sibling directory with one local modification (Menlo font patch) |
| **License** | AGPL-3.0, inherited from mk48 |

## Build & run locally

You need [rustup](https://rustup.rs) and `gmake`/`gcc` if they're not already
installed. The project pins **Rust nightly-2024-04-20** and needs both the
`wasm32-unknown-unknown` target and [trunk](https://trunkrs.dev) for the
client build.

```bash
# First-time toolchain setup
rustup toolchain install nightly-2024-04-20 --profile minimal
rustup default nightly-2024-04-20
rustup target add wasm32-unknown-unknown
cargo install --locked trunk --version 0.21.7

# Clone both this repo and the kodiak engine side-by-side
git clone https://github.com/scotthurff/warships.git
git clone https://github.com/scotthurff/warships-kodiak.git
# ↑ Cargo.toml path deps assume warships/ and warships-kodiak/ are siblings

# Build the WASM client bundle
cd warships/client
make release

# Run the server (which also serves the client bundle embedded via minicdn)
cd ../server
cargo run --release -- \
  --client-authenticate-burst 999999 \
  --http-bandwidth-limit 99999999
```

Play at `https://localhost:8443/` — Safari / Chrome will show a self-signed
cert warning; click through it.

### Playing from an iPad on the same Wi-Fi

Find your Mac's LAN IP:

```bash
ipconfig getifaddr en0    # Wi-Fi; use en1 on some machines
```

Then open `https://192.168.X.X:8443/` on the iPad. Same cert warning — tap
**Show Details** → **visit this website** → **Visit Website**. If it won't
connect at all, macOS firewall may be blocking the server binary: System
Settings → Network → Firewall → Options → add
`warships/server/target/release/server` with "Allow incoming connections".

## Deploying to Fly.io

The repo ships with a Dockerfile, `fly.toml`, and a `deploy.sh` script that
stages `warships/` and `warships-kodiak/` into a temp build context before
running `fly deploy` (Cargo.toml path deps need both repos together).

```bash
# One-time setup
brew install flyctl
fly auth login

# Reserve the app name using the fly.toml in this repo
cd warships
fly launch --no-deploy --copy-config --name warships

# Deploy
./deploy.sh
```

First build takes 10-20 minutes (downloading nightly toolchain, compiling
~800 crates). Subsequent deploys cache layers and take 2-3 minutes.

**About the machine**: the `fly.toml` asks for a `shared-cpu-1x / 512MB`
machine with `min_machines_running = 1` and `auto_stop_machines = "off"`. The
game loop keeps running even with zero players, because match state and bot
AI can't survive a cold stop. Rough cost: ~$3-5/month.

## Project layout

```
warships/
├── client/             # Rust/WASM client (Yew UI + WebGL renderer)
│   ├── src/
│   │   ├── game.rs             # main loop, input, WebGL render pass
│   │   ├── camera.rs           # zoom / follow logic
│   │   ├── state.rs            # client-side game state
│   │   └── ui/                 # Yew components
│   │       ├── game_ui.rs             # root UI orchestrator
│   │       ├── ship_picker.rs         # Capture the Area title screen
│   │       ├── countdown_overlay.rs   # 3-2-1-FIGHT intro
│   │       ├── cta_respawn_overlay.rs # post-death respawn UI
│   │       ├── match_end_overlay.rs   # results + stats table
│   │       ├── minimap.rs             # full-arena minimap
│   │       └── touch_controls.rs      # iPad on-screen controls
│   └── Makefile        # `make release` → trunk build
├── server/             # Rust server (physics, bots, match state)
│   ├── src/
│   │   ├── server.rs           # main tick loop, CTA orchestration
│   │   ├── match_state.rs      # 5-minute match FSM + capture mechanics
│   │   ├── bot.rs              # AI: engagement, objective push, fleet loadouts
│   │   ├── world.rs            # arena + entity pools
│   │   ├── world_spawn.rs      # pentagon spawn slots
│   │   ├── world_inbound.rs    # Spawn command handler with CTA override
│   │   └── player.rs           # TempPlayer struct with match_team + match_slot
│   └── main.rs         # minicdn bundles client/dist/ into the binary
├── common/             # Shared types (protocol, entities, math)
├── macros/             # Procedural macros for entity definitions
├── plans/              # Design docs and work plans
├── Dockerfile          # Multi-stage build (builder + slim runtime)
├── fly.toml            # Fly.io config
├── deploy.sh           # Stages warships/ + warships-kodiak/ → fly deploy
├── CLAUDE.md           # Conventions for Claude Code sessions
└── decisions.md        # Running log of architectural decisions
```

## License

AGPL-3.0, same as mk48. **If you deploy this to a public network, you must
make the source available to players of the hosted version.** That's why
both this repo and [warships-kodiak](https://github.com/scotthurff/warships-kodiak)
are public.

## Credits

- [Softbear Studios](https://github.com/SoftbearStudios) — original [mk48.io](https://github.com/SoftbearStudios/mk48) and the [kodiak](https://github.com/softbearstudios/kodiak) engine
- Bot AI overhaul, Capture the Area mode, touch controls, design system, and deployment — this fork
