# Scanner image for https://github.com/XOXNO/rs-lending-xlm.
#
# The audit runs offline, so this build installs the pinned toolchain and stellar-cli, fetches every crate, builds the
# contract WASM that the integration harness loads, and compiles every test. threat_model.md, next to this file, says
# how to run them. The scanner gives the whole build 45 minutes, so only one small test binary runs here.
# Builds on x86-64 and arm64 Linux: install-stellar-cli.sh pins a release for each.

# The Rust version that rust-toolchain.toml pins, so rustup adds only its targets and components below.
FROM rust:1.95-bookworm
ENV DEBIAN_FRONTEND=noninteractive
RUN sed -i 's|http://deb.debian.org|https://deb.debian.org|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update && apt-get install -y --no-install-recommends python3 libdbus-1-3 libudev1 \
    && rm -rf /var/lib/apt/lists/*

# scanner contract: the checkout lives inside the image, at /src
COPY . /src
WORKDIR /src

# The targets and components that rust-toolchain.toml pins (rustup reads the file), and the stellar-cli release CI
# uses (STELLAR_VERSION in .github/workflows/tests.yml), checked against its pinned SHA-256.
ENV PATH="/root/.local/bin:${PATH}"
RUN rustup toolchain install \
    && rustc --version \
    && STELLAR_VERSION=28.0.0 bash .github/scripts/install-stellar-cli.sh

# An ARG, so that it bounds the memory this build uses without reaching the audit environment.
ARG CARGO_BUILD_JOBS=8

# Every crate the workspace depends on, for every target. tests/fuzz and services/ are separate workspaces and are
# not built.
RUN cargo fetch

# The contract WASM (target/wasm32v1-none/release, which the harness reads, and target/optimized), then every test
# binary with the features CI uses. A step that fails fails the build, and primary_contact gets the log.
RUN make build \
    && cargo test --workspace --features price-aggregator/testing --no-run

# Run the smoke binary so the log shows the harness deploys the protocol, without letting a failure abort the build.
RUN timeout 600 cargo test -p test-harness --test smoke_test \
    || echo "WARNING: smoke_test reported failures"

# Everything is fetched. From here on neither cargo nor rustup may wait for a server it cannot reach.
ENV CARGO_NET_OFFLINE=true \
    RUSTUP_AUTO_INSTALL=0
