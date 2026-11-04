# Komet prover image = compiled semantics image (semantics.Dockerfile)
# + Kore/Booster backend (backend.Dockerfile) + Z3 official release.
# scripts/build-images.sh builds the two inputs first and passes their tags.

ARG BACKEND_IMAGE
ARG SEMANTICS_IMAGE
FROM ${BACKEND_IMAGE} AS backend

FROM ${SEMANTICS_IMAGE}
ARG Z3_VERSION
ARG Z3_ZIP_SHA256
ARG Z3_BIN_SHA256
ARG BOOSTER_FLAGS

# Z3 official release build (the booster drives z3 as an external process).
RUN curl -fsSL -o /tmp/z3.zip \
      "https://github.com/Z3Prover/z3/releases/download/z3-${Z3_VERSION}/z3-${Z3_VERSION}-x64-glibc-2.35.zip" \
 && echo "${Z3_ZIP_SHA256}  /tmp/z3.zip" | sha256sum -c - \
 && python3 -c "import zipfile; zipfile.ZipFile('/tmp/z3.zip').extractall('/tmp/z3')" \
 && install -D -m 0755 "/tmp/z3/z3-${Z3_VERSION}-x64-glibc-2.35/bin/z3" /opt/z3/bin/z3 \
 && install -D -m 0644 "/tmp/z3/z3-${Z3_VERSION}-x64-glibc-2.35/LICENSE.txt" /opt/z3/LICENSE.txt \
 && echo "${Z3_BIN_SHA256}  /opt/z3/bin/z3" | sha256sum -c - \
 && rm -rf /tmp/z3 /tmp/z3.zip

# Backend: the real binary plus a wrapper that adds the recorded proof flags.
COPY --from=backend /out /opt/komet-backend
RUN mkdir -p /opt/komet-backend/wrapper \
 && printf '#!/bin/sh\nexec /opt/komet-backend/bin/kore-rpc-booster "$@" %s\n' "${BOOSTER_FLAGS}" \
      > /opt/komet-backend/wrapper/kore-rpc-booster \
 && chmod 0755 /opt/komet-backend/wrapper/kore-rpc-booster \
 && sed 's#/out/#/opt/komet-backend/#' /opt/komet-backend/kore-rpc-booster.sha256 | sha256sum -c -
ENV PATH=/opt/komet-backend/wrapper:/opt/z3/bin:/opt/komet/.venv/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# Record the toolchain identity inside the image.
RUN { echo "k=$(kompile --version | head -1)"; \
      echo "z3=$(z3 --version)"; \
      echo "z3_path=$(command -v z3)"; \
      echo "booster_path=$(command -v kore-rpc-booster)"; \
      echo "booster_commit=$(cat /opt/komet-backend/haskell-backend.commit)"; \
      echo "booster_sha256=$(cut -d' ' -f1 /opt/komet-backend/kore-rpc-booster.sha256)"; \
      echo "booster_flags=${BOOSTER_FLAGS}"; \
      echo "komet_commit=$(git -C /opt/komet rev-parse HEAD)"; \
      echo "stellar=$(stellar --version | head -1)"; \
      echo "python=$(/opt/komet/.venv/bin/python --version)"; \
      echo "pyk_dir=$(/opt/komet/.venv/bin/python -c 'import pyk,os;print(os.path.dirname(pyk.__file__))')"; \
      echo "kdist_dir=$(ls -d /opt/komet-cache/kdist-*)"; \
    } > /opt/toolchain.txt \
 && find /opt/komet-cache \( -name definition.kore -o -name interpreter -o -name 'interpreter.so' \) \
      | sort | xargs sha256sum > /opt/semantics.sha256 \
 && cat /opt/toolchain.txt /opt/semantics.sha256
WORKDIR /work
