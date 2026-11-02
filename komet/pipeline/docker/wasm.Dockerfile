# Harness Wasm builder: pinned Rust 1.95.0 image + Stellar CLI 28.0.0.
# Builds the komet/ harness crates; a rebuild must match the recorded SHA-256.
ARG RUST_IMAGE
FROM ${RUST_IMAGE}
ARG STELLAR_VERSION
ARG STELLAR_SHA256
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates libdbus-1-3 \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32v1-none \
 && curl -fsSL -o /tmp/stellar.tgz \
      "https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_VERSION}/stellar-cli-${STELLAR_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
 && echo "${STELLAR_SHA256}  /tmp/stellar.tgz" | sha256sum -c - \
 && mkdir -p /tmp/stellar && tar -xzf /tmp/stellar.tgz -C /tmp/stellar \
 && install -m 0755 /tmp/stellar/stellar /usr/local/bin/stellar \
 && rm -rf /tmp/stellar /tmp/stellar.tgz \
 && rustc --version && stellar --version
# rust-toolchain.toml names channel 1.95 plus host components; use the image's
# exact 1.95.0 toolchain instead of letting rustup resolve the channel.
ENV RUSTUP_TOOLCHAIN=1.95.0
WORKDIR /src
