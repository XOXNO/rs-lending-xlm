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

COPY Cargo.toml ./
COPY src ./src
COPY config ./config

RUN --mount=type=cache,target=/root/.cargo/registry,sharing=locked \
    --mount=type=cache,target=/build/target,sharing=locked \
    cargo build --release --bin lending-exporter \
    && cp target/release/lending-exporter /lending-exporter

# ------------------------------------------------------------------------
# Runtime stage — distroless cc (provides libgcc, libstdc++, openssl).
# ------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot

COPY --from=builder /lending-exporter /usr/local/bin/lending-exporter
COPY config /etc/lending-exporter

USER nonroot:nonroot
EXPOSE 9110

ENV EXPORTER_CONFIG=/etc/lending-exporter/testnet.yaml \
    RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/lending-exporter"]
