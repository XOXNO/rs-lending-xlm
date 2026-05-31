# syntax=docker/dockerfile:1.7

# ------------------------------------------------------------------------
# Build stage
# ------------------------------------------------------------------------
FROM rust:1.84-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies separately from source.
COPY Cargo.toml ./
COPY src ./src
COPY config ./config

# Build with BuildKit cache mounts.
RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --bin keeper-bot \
    && cp target/release/keeper-bot /keeper-bot

# ------------------------------------------------------------------------
# Runtime stage — distroless cc (provides libgcc, libstdc++, openssl).
# ------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /keeper-bot /usr/local/bin/keeper-bot
COPY config /etc/keeper

USER nonroot:nonroot
EXPOSE 9090

ENV KEEPER_NETWORK=testnet \
    KEEPER_CONFIG=/etc/keeper/testnet.yaml \
    RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/keeper-bot"]
