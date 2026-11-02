# Komet fork at a pinned commit, its locked Python environment (with the
# branch's vendored pyk overlay) and the compiled Soroban semantics. Inputs
# are pinned by ../pins.env.

ARG K_IMAGE
FROM ${K_IMAGE}
ARG UV_VERSION
ARG UV_SHA256
ARG STELLAR_VERSION
ARG STELLAR_SHA256
ARG KOMET_REPO
ARG KOMET_COMMIT
ARG KOMET_LEMMAS_SHA256
ARG KOMET_PROOF_PY_SHA256
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

# Source identity of the proof-selected semantics lemmas and runner.
RUN cd /opt/komet \
 && echo "${KOMET_LEMMAS_SHA256}  src/komet/kdist/soroban-semantics/komet-lemmas.md" | sha256sum -c - \
 && echo "${KOMET_PROOF_PY_SHA256}  src/komet/proof.py" | sha256sum -c -

# pyk overlay (vendor/pyk-overlay in the Komet branch): replace the two
# changed files of the installed kframework 7.1.337 package in place, checking
# the stock hashes before and the selected hashes after.
RUN P=/opt/komet/.venv/lib/python3.10/site-packages/pyk \
 && O=/opt/komet/vendor/pyk-overlay/pyk \
 && printf '%s  %s\n' \
      ddc9aa9abff1b180cf8276375647469b83cfd3cba5f385cbd899e0529431d47f "$P/konvert/_kast_to_kore.py" \
      dfc1f86ef0e8cae44c5833efd4b0d613ee90ff4d7747e0663d3c575b0191fd30 "$P/proof/reachability.py" \
    | sha256sum -c - \
 && cp "$O/konvert/_kast_to_kore.py" "$P/konvert/_kast_to_kore.py" \
 && cp "$O/proof/reachability.py" "$P/proof/reachability.py" \
 && printf '%s  %s\n' \
      fdd855b4d4280f8739ffb5963f3d4997756ec44585f4b23ae94cd85f5a75463a "$P/konvert/_kast_to_kore.py" \
      057fcc12b91f8f7ce355c74626e79df83b6f6623ba8772c2e9813588fd3ef3dd "$P/proof/reachability.py" \
    | sha256sum -c - \
 && find "$P" -name __pycache__ -prune -exec rm -r {} +
ENV PATH=/opt/komet/.venv/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    PYTHONDONTWRITEBYTECODE=1

# Compiled semantics used by `komet prove`: haskell (symbolic), llvm (concrete
# deployment) and llvm-library (Booster's simplification library). -j1 keeps
# the LLVM compilations sequential (two in parallel exceed 30 GB RAM).
RUN cd /opt/komet \
 && komet-kdist -v build -j1 soroban-semantics.haskell soroban-semantics.llvm soroban-semantics.llvm-library \
 && chmod -R a+rX /opt/komet-cache
