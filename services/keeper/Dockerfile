





FROM rust:1.95-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build


COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY config ./config


RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --locked --bin keeper-bot \
    && cp target/release/keeper-bot /keeper-bot




FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /keeper-bot /usr/local/bin/keeper-bot
COPY config /etc/keeper

USER nonroot:nonroot
EXPOSE 9090

# The network comes from the `network` field of the file KEEPER_CONFIG points
# at; there is no separate network variable, so nothing here can disagree with
# the config.
ENV KEEPER_CONFIG=/etc/keeper/mainnet.yaml \
    RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/keeper-bot"]
