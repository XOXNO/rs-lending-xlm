# syntax=docker/dockerfile:1.7

# ------------------------------------------------------------------------
# Build stage
#
# `mx-keyvault` is fetched from the private XOXNO/mx-chain-rust git repo.
# Pass git credentials with:
#   DOCKER_BUILDKIT=1 docker build \
#     --secret id=git_credentials,src=$HOME/.git-credentials \
#     -t keeper-bot:latest .
# Or override with `--ssh default` if you prefer SSH-based auth.
# ------------------------------------------------------------------------
FROM rust:1.84-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies separately from source.
COPY Cargo.toml ./
COPY src ./src
COPY config ./config

# Build with BuildKit cache mounts + git-credentials secret.
RUN --mount=type=secret,id=git_credentials,target=/root/.git-credentials \
    --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    git config --global credential.helper 'store --file=/root/.git-credentials' \
    && cargo build --release --bin keeper-bot \
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
