# Kore/Booster backend built from a pinned XOXNO/haskell-backend commit.
# Output: /out/bin/kore-rpc-booster, /out/haskell-backend.commit,
#         /out/kore-rpc-booster.sha256 (copied into the prover image).
# Inputs are pinned by ../pins.env (see scripts/build-images.sh).

ARG K_IMAGE

# Stage 1: Haskell toolchain and backend dependencies (cached across commits).
FROM ${K_IMAGE} AS backend-deps
ARG STACK_VERSION
ARG STACK_SHA256
ARG HASKELL_BACKEND_REPO
ARG HASKELL_BACKEND_DEPS_COMMIT
USER root
ENV DEBIAN_FRONTEND=noninteractive STACK_ROOT=/opt/stack LANG=C.UTF-8
RUN apt-get update \
 && apt-get install -y --no-install-recommends libsecp256k1-dev libnuma-dev \
 && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL -o /tmp/stack.tgz \
      "https://github.com/commercialhaskell/stack/releases/download/v${STACK_VERSION}/stack-${STACK_VERSION}-linux-x86_64.tar.gz" \
 && echo "${STACK_SHA256}  /tmp/stack.tgz" | sha256sum -c - \
 && tar -xzf /tmp/stack.tgz -C /tmp \
 && install -m 0755 "/tmp/stack-${STACK_VERSION}-linux-x86_64/stack" /usr/local/bin/stack \
 && rm -rf /tmp/stack.tgz "/tmp/stack-${STACK_VERSION}-linux-x86_64"
RUN git clone --filter=blob:none "${HASKELL_BACKEND_REPO}" /src/haskell-backend \
 && cd /src/haskell-backend \
 && git checkout --detach "${HASKELL_BACKEND_DEPS_COMMIT}" \
 && test "$(git rev-parse HEAD)" = "${HASKELL_BACKEND_DEPS_COMMIT}"
# GHC comes from Stack's setup-info, which verifies the GHC bindist hash;
# every Hackage package is pinned by stack.yaml.lock (cabal-file hash + tree).
RUN cd /src/haskell-backend \
 && stack --no-terminal --no-system-ghc setup \
 && stack --no-terminal --no-system-ghc build -j4 --only-dependencies hs-backend-booster:exe:kore-rpc-booster

# Stage 2: build kore-rpc-booster at the selected backend commit.
FROM backend-deps AS backend
ARG HASKELL_BACKEND_COMMIT
RUN cd /src/haskell-backend \
 && git fetch origin "${HASKELL_BACKEND_COMMIT}" \
 && git checkout --detach "${HASKELL_BACKEND_COMMIT}" \
 && test "$(git rev-parse HEAD)" = "${HASKELL_BACKEND_COMMIT}" \
 && test -z "$(git status --porcelain)" \
 && stack --no-terminal --no-system-ghc build -j2 hs-backend-booster:exe:kore-rpc-booster \
 && mkdir -p /out/bin \
 && cp "$(stack --no-system-ghc path --local-install-root)/bin/kore-rpc-booster" /out/bin/ \
 && git rev-parse HEAD > /out/haskell-backend.commit \
 && sha256sum /out/bin/kore-rpc-booster > /out/kore-rpc-booster.sha256
