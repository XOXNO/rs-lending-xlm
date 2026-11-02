# Komet fork at a pinned commit, its locked Python environment (with the
# vendored pyk patch) and the compiled Soroban semantics. Inputs are pinned by
# ../pins.env; the build context is the repository's komet/ directory.

ARG K_IMAGE
FROM ${K_IMAGE}
ARG UV_VERSION
ARG UV_SHA256
ARG STELLAR_VERSION
ARG STELLAR_SHA256
ARG KOMET_REPO
ARG KOMET_COMMIT
USER root
ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8 KPROFILE_TELEMETRY_DISABLED=true

# uv (pinned release) and Stellar CLI (same pin as .github/scripts/install-stellar-cli.sh).
RUN curl -fsSL -o /tmp/uv.tgz \
      "https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-x86_64-unknown-linux-gnu.tar.gz" \
 && echo "${UV_SHA256}  /tmp/uv.tgz" | sha256sum -c - \
 && tar -xzf /tmp/uv.tgz -C /tmp \
 && install -m 0755 /tmp/uv-x86_64-unknown-linux-gnu/uv /usr/local/bin/uv \
 && rm -rf /tmp/uv.tgz /tmp/uv-x86_64-unknown-linux-gnu \
 && curl -fsSL -o /tmp/stellar.tgz \
      "https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_VERSION}/stellar-cli-${STELLAR_VERSION}-x86_64-unknown-linux-gnu.tar.gz" \
 && echo "${STELLAR_SHA256}  /tmp/stellar.tgz" | sha256sum -c - \
 && mkdir -p /tmp/stellar && tar -xzf /tmp/stellar.tgz -C /tmp/stellar \
 && install -m 0755 /tmp/stellar/stellar /usr/local/bin/stellar \
 && rm -rf /tmp/stellar /tmp/stellar.tgz

# Komet fork at a pinned commit, with its locked Python environment.
ENV UV_PYTHON_DOWNLOADS=never UV_CACHE_DIR=/tmp/uv-cache \
    XDG_CACHE_HOME=/opt/komet-cache
RUN git clone --filter=blob:none "${KOMET_REPO}" /opt/komet \
 && cd /opt/komet \
 && git checkout --detach "${KOMET_COMMIT}" \
 && test "$(git rev-parse HEAD)" = "${KOMET_COMMIT}" \
 && test -z "$(git status --porcelain)" \
 && uv sync --locked --python /usr/bin/python3.10 --no-dev \
 && rm -rf /tmp/uv-cache

# pyk patch (komet/vendor/pyk-7.1.337): checks stock and patched file hashes.
COPY vendor/pyk-7.1.337 /opt/komet-vendor/pyk-7.1.337
RUN /opt/komet-vendor/pyk-7.1.337/apply.sh /opt/komet/.venv/lib/python3.10/site-packages/pyk
ENV PATH=/opt/komet/.venv/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    PYTHONDONTWRITEBYTECODE=1

# Compiled semantics used by `komet prove`: haskell (symbolic), llvm (concrete
# deployment) and llvm-library (Booster's simplification library). -j1 keeps
# the LLVM compilations sequential (two in parallel exceed 30 GB RAM).
RUN cd /opt/komet \
 && komet-kdist -v build -j1 soroban-semantics.haskell soroban-semantics.llvm soroban-semantics.llvm-library \
 && chmod -R a+rX /opt/komet-cache
