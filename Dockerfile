# xferrum builder image — analogous to xcaddy for Caddy.
#
# Usage in your own Dockerfile:
#
#   FROM ghcr.io/your-org/xferrum:latest AS builder
#   COPY ./plugins /build/plugins
#   RUN xferrum build --output /ferrum
#
#   FROM debian:bookworm-slim
#   COPY --from=builder /ferrum /usr/local/bin/ferrum
#   COPY ferrum.toml /etc/ferrum/ferrum.toml
#   CMD ["ferrum", "--config", "/etc/ferrum/ferrum.toml"]

# ── Stage 1: compile xferrum ────────────────────────────────────────────────
FROM rust:1-slim AS xferrum-builder

WORKDIR /src
COPY . .
RUN cargo build --release -p xferrum

# ── Stage 2: builder image ───────────────────────────────────────────────────
FROM rust:1-slim

COPY --from=xferrum-builder /src/target/release/xferrum /usr/local/bin/xferrum

# Ferrum workspace sources — used as path dependency when compiling plugins.
COPY --from=xferrum-builder /src /opt/ferrum-workspace

# Pre-warm the cargo registry so plugin builds start from cached deps.
RUN cargo fetch --manifest-path /opt/ferrum-workspace/Cargo.toml

ENV FERRUM_PATH=/opt/ferrum-workspace/ferrum
WORKDIR /build

ENTRYPOINT ["xferrum", "build"]
CMD ["--output", "/usr/local/bin/ferrum"]
