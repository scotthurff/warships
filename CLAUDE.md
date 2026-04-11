# CLAUDE.md — WARSHIPS

## Project

WARSHIPS is a fork of [mk48.io](https://github.com/SoftbearStudios/mk48) adapted into a kid-friendly single-player naval combat game for iPad and desktop browser.

**Tech:** Rust server + Rust/WASM client (Yew framework for UI, WebGL for rendering, kodiak game engine)
**License:** AGPL-3.0 (inherited from mk48). Source must be available to players of any hosted deployment.
**Repo:** github.com/scotthurff/warships
**Engine dep:** github.com/scotthurff/warships-kodiak (fork of softbearstudios/kodiak, vendored as sibling directory)

## Build & Run

```bash
# First time: install toolchain
rustup override set nightly-2024-04-20
rustup target add wasm32-unknown-unknown
cargo install --locked trunk --version 0.21.7

# Build client (WASM)
cd client && make release

# Run server (also serves client)
cd server && cargo run -- --client-authenticate-burst 999999 --http-bandwidth-limit 99999999

# Play at https://localhost:8443/ (click through cert warning)
```

## Key Directories

- `client/src/game.rs` — main game loop, input handling, rendering
- `client/src/ui/` — Yew UI components (HTML overlay)
- `client/src/ui/touch_controls.rs` — touch control overlay (our addition)
- `client/src/ui/game_ui.rs` — main UI orchestrator (spawn screen, HUD)
- `client/src/ui/logo.rs` — WARSHIPS logo
- `client/src/armament.rs` — weapon selection logic
- `server/src/bot.rs` — bot AI (difficulty tuning here)
- `common/src/lib.rs` — game constants (name, domain)
- `common/src/entity/_type.rs` — all 45 ship definitions
- `client/index.html` — HTML entry, fonts, meta tags

## Decision Log

**All significant decisions must be logged in `decisions.md` at the project root.** Format:

```markdown
## YYYY-MM-DD: Decision Title

**Context:** Why this decision came up
**Decision:** What was decided
**Alternatives considered:** What else was evaluated
**Consequences:** What this means going forward
```

Log decisions about: architecture changes, library choices, feature scope, platform choices, things we tried and abandoned, and anything a future session would need to know.

## Conventions

- **Font:** Black Ops One for logo/title only. Menlo/SF Mono monospace for HUD/controls.
- **Touch targets:** 80pt minimum for primary actions, 60pt for secondary.
- **Bot difficulty:** Easy mode is the default. Tune for ages 5-10.
- **No multiplayer UI:** No social links, leaderboards, chat, teams, nicknames. Single-player with bots.
- **Build cycle:** Edit Rust → `cd client && make release` → restart server → hard refresh browser.
- **Kodiak framework:** External dependency, can't modify directly. Work around with CSS/JS overrides or by replacing kodiak components with our own (like we did with SpawnOverlay).

## Target Audience

Kids 10 and under playing on iPad (Safari) and desktop browser. Controls must work with touch. UI must be simple and readable without text.
