# Multi-stage Docker build for Warships (mk48.io fork)
#
# Stage 1 builds the WASM client (trunk → /src/warships/client/dist) and
# the Rust server binary. The server embeds the built client bundle at
# compile time via `minicdn::release_include_mini_cdn!("../../client/dist/")`.
#
# Stage 2 is a minimal runtime image that just runs the server binary.
#
# IMPORTANT: this Dockerfile expects the build context to contain BOTH
# the `warships/` directory and its sibling `warships-kodiak/`. The
# `deploy.sh` script in this directory stages both into a temporary
# context before running `fly deploy`, which is the intended workflow.

# ─── Stage 1: builder ────────────────────────────────────────────────
FROM rust:1.78-bookworm AS builder

ARG RUST_NIGHTLY=nightly-2024-04-20

# System deps for a full Rust build. libssl-dev for openssl-sys, git for
# any Cargo.toml `git = "..."` patches.
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        git \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Pin to the exact nightly the project uses (per warships/CLAUDE.md).
RUN rustup toolchain install "${RUST_NIGHTLY}" --profile minimal \
    && rustup default "${RUST_NIGHTLY}" \
    && rustup target add wasm32-unknown-unknown

# Install trunk for the WASM client build.
RUN cargo install --locked trunk --version 0.21.7

WORKDIR /src

# Copy both repos. Cargo.toml path deps (`../../warships-kodiak/...`)
# resolve inside this layout because warships/ and warships-kodiak/
# are siblings under /src, exactly as they are on disk.
COPY warships /src/warships
COPY warships-kodiak /src/warships-kodiak

# Build the client. `make release` runs trunk with --release --minify
# and writes the WASM + JS + HTML bundle to client/dist/.
WORKDIR /src/warships/client
RUN make release

# Build the server. The server reads client/dist/ at compile time via
# the minicdn macro, so the client MUST be built first.
WORKDIR /src/warships/server
RUN cargo build --release --bin server

# ─── Stage 2: runtime ────────────────────────────────────────────────
FROM debian:bookworm-slim

# ca-certificates lets the server hit external IP-detection services
# (icanhazip.com, ifconfig.me) on startup. Everything else is statically
# linked into the Rust binary.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/warships/server/target/release/server /usr/local/bin/server

# Expose only the HTTP port — Fly's edge handles TLS termination and
# forwards cleartext to this port internally. The server's internal
# HTTPS listener on 8443 is unused when deployed.
EXPOSE 8080

# Generous auth burst + bandwidth limits mirror the local dev command.
# The server's own TLS is disabled since Fly handles cert at the edge;
# if kodiak's HTTPS bind fails (self-signed gen issues etc) it logs
# an error but continues serving over HTTP, which is what we want.
CMD ["server", "--client-authenticate-burst", "999999", "--http-bandwidth-limit", "99999999"]
