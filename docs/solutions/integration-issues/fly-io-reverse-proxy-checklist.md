---
title: Deploying an upstream-assuming game server behind Fly.io's edge
category: integration-issues
component: deploy/fly-io
problem_type: reverse_proxy_mismatch
symptoms:
  - "Health checks fail with 308 Permanent Redirect"
  - "WebSocket upgrade returns 401 invalid origin from the browser"
  - "WebSocket connects, then immediately 'loses server connection' on Start Game"
  - "Client attempts wss://host:8062/ws and gets ECONNRESET"
severity: high
resolution: fixed
fixed_in:
  - warships/a849ada (initial Fly config)
  - warships/71e5905 (internal_port 8080 → 80)
  - warships-kodiak/52d1957 (HTTP no longer redirects)
  - warships/67e1fea (allow fly.dev as WS origin)
  - warships-kodiak/ea5a03c (don't rewrite port when host has none)
related_files:
  - fly.toml
  - Dockerfile
  - deploy.sh
  - ../warships-kodiak/server/src/entry_point.rs
  - ../warships-kodiak/server/src/router.rs
  - ../warships-kodiak/client/src/broker/client_context.rs
  - common/src/lib.rs
tags: [fly-io, websockets, reverse-proxy, tls-termination, deployment]
---

# Deploying an upstream-assuming game server behind Fly.io's edge

Kodiak (the game engine underlying WARSHIPS) was built for bare-metal and
multi-shard deploys where each server terminates its own TLS and owns its
own ports. Fly.io's model is the opposite: the edge terminates TLS and only
80/443 are reachable from outside the machine. Every assumption kodiak makes
about "my port", "my TLS", and "my multi-server scheme" had to be relaxed.

This entry captures the full chain of fixes needed to stand up a kodiak-
based game on Fly.io. Any one of these fixes alone gets you a different
failure mode — the whole chain has to land.

## The five failure modes, in order encountered

### 1. Container listens on the wrong internal port

**Symptom:** `fly deploy` succeeds, machine spins up, then Fly kills it:
> Health check failed: connection refused

**Cause:** Kodiak binds **80/443 when it has `CAP_NET_BIND_SERVICE`**, and
**8080/8443 otherwise**. Locally you're unprivileged so you see
`HTTP port: 8080, HTTPS port: 8443` in logs. Inside the Fly container you
run as root, so kodiak binds 80/443 directly. `fly.toml` defaulted to
`internal_port = 8080` — nothing was listening there.

**Fix:** `fly.toml`:
```toml
[[services]]
  internal_port = 80     # was 8080
  protocol      = "tcp"
```

Verify inside the machine:
```
fly ssh console -a warships
ss -tlnp   # expect LISTEN 0.0.0.0:80, not :8080
```

### 2. Kodiak 308-redirects every HTTP request to HTTPS

**Symptom:** Health check upgraded to port 80 now succeeds the TCP
handshake — but Fly treats 308 as failure:
> Health check failed: HTTP 308 Permanent Redirect

**Cause:** `warships-kodiak/server/src/entry_point.rs` in **release builds
only** replaces the normal axum router on port 80 with a stub that 308s
everything to `https://<host>/`. Intended for bare-metal where the same
process handles both ports; catastrophic behind Fly, because Fly's edge
forwards plaintext HTTP to internal port 80 after terminating TLS. The
redirect bounces the user back out through Fly's edge to the same URL
they already requested.

**Fix:** patch kodiak to serve the real app on HTTP too. Debug builds
already did this; release now matches:
```rust
// warships-kodiak/server/src/entry_point.rs — 52d1957
let http_app = app.clone();   // was a 34-line redirect router
```

We keep this as a kodiak fork patch because upstream's assumption is
correct for their deploy model — we just need a different behavior.

### 3. WebSocket upgrade is rejected by origin check

**Symptom:** Page loads over HTTPS. HTML/WASM download fine. The instant
the client opens its first `/ws` connection, the browser console shows:
> WebSocket connection failed — HTTP 401

Server logs:
> invalid origin

**Cause:** Kodiak's `check_origin` in `router.rs` allows a WebSocket
upgrade only if the browser's `Origin` header matches one of:
- `GAME_CONSTANTS.domain`
- `softbear.com`
- a known-port localhost/127.0.0.1

`common/src/lib.rs` had `domain: "localhost"`, so any browser connecting
from `https://warships.fly.dev/` was rejected.

**Fix:** set the real domain:
```rust
// common/src/lib.rs
pub const GAME_CONSTANTS: GameConstants = GameConstants {
    name:   "Warships",
    domain: "warships.fly.dev",   // was "localhost"
    // ...
};
```

Local dev still works because `router.rs` has a separate branch for
`:8443` / `:8080` / `localhost` origins.

### 4. Client rewrites WebSocket URL to a non-routed port

**Symptom:** First WebSocket connects fine. Then the player clicks
**Start Game**, and the client's second WebSocket connect goes to
`wss://warships.fly.dev:8062/ws` → `ECONNRESET` → "Connection lost".

**Cause:** Kodiak supports multi-server shards where `server_id = N`
maps to `<host>:{8000 + N}`. The client *rewrites every WebSocket URL*
to that shard's port after reading `/system.json`. Before the first
`system.json` read, `server_id` is `None` and the client falls through
to `window.location.host` verbatim — that's why the first connect works.

Behind Fly only 80/443 exist, so 8062 is unreachable.

**Fix:** `warships-kodiak/client/src/broker/client_context.rs` — if the
host reported by the server has no `:port` suffix, use it verbatim:
```rust
if !host.contains(':') {
    (encryption, host)
} else {
    // existing multi-server port-rewrite logic
}
```

Multi-shard deploys always report host:port so upstream behavior is
preserved.

### 5. AGPL compliance — kodiak fork must be public

**Not a failure mode; a licensing requirement.** Kodiak is AGPL-3.0.
Deploying a hosted version means the source users interact with must
be reachable. Two of the above fixes live in the vendored kodiak fork
(`52d1957`, `ea5a03c`), so the public kodiak repo has to track every
production commit.

**Rule:** after any kodiak change that makes it to prod, push
`warships-kodiak` to origin before (or at latest with) the warships
deploy. Don't ship without the source matching.

## Verification checklist (re-deploy sanity)

After any change that touches deploy config or kodiak-level networking:

```
# 1. Container listens on the port Fly probes
fly ssh console -a warships -C 'ss -tlnp | grep -E ":80|:443"'

# 2. HTTP on port 80 returns 200, not 308
curl -si http://warships.fly.dev/ | head -1    # HTTP/1.1 200

# 3. WebSocket upgrade passes origin check (use a Node script)
node -e "const WS=require('ws'); \
  const w=new WS('wss://warships.fly.dev/ws',{origin:'https://warships.fly.dev'}); \
  w.on('open',()=>{console.log('ok');w.close()}); \
  w.on('error',e=>console.log('err',e.message));"

# 4. /system.json.host, if present, is the unchanged fly.dev host
curl -s https://warships.fly.dev/system.json | jq .
```

## Prevention

- **Know your hosting model before forking an engine.** Kodiak's self-TLS
  + multi-shard-port scheme is not wrong — it's wrong *for Fly's edge
  model*. Any time you adopt a framework built for a different deploy
  topology, audit the networking assumptions upfront: port choice, TLS
  termination, Host vs Forwarded headers, origin check, shard URL
  scheme.
- **Release ≠ debug.** Two of the five bugs only reproduce in release
  (`cargo run --release` behind `trunk build --release`). Always
  reproduce locally in release mode before the first deploy.
- **Failure modes are ordered.** You cannot diagnose #3 until #2 is
  fixed, or #4 until #3 is fixed. When a new hosting target has a
  chain of issues, don't jump between them — walk the chain from
  "TCP handshake?" → "HTTP 200?" → "WS upgrade?" → "WS stays up?".
  Skipping ahead wastes time misreading error messages from an
  earlier broken layer.
- **Hard-coded `"localhost"` in a production constant will bite you.**
  Grep for it before any first deploy.
