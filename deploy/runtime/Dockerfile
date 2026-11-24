ARG LENS_BUILD_IMAGE=cgr.dev/chainguard/wolfi-base@sha256:ae9575b121f5776e2f04e192064bc2c5665af9d1c425d620dc773aca31698712
ARG LENS_RUNTIME_IMAGE=cgr.dev/chainguard/wolfi-base@sha256:ae9575b121f5776e2f04e192064bc2c5665af9d1c425d620dc773aca31698712
ARG LENS_UI_BUILD_IMAGE=node:24.14.1-bookworm-slim@sha256:b506e7321f176aae77317f99d67a24b272c1f09f1d10f1761f2773447d8da26c

FROM $LENS_UI_BUILD_IMAGE AS ui-builder
WORKDIR /src
ENV NEXT_TELEMETRY_DISABLED=1
COPY package.json package-lock.json ./
COPY src/ui/lib/package.json src/ui/lib/package.json
COPY src/ui/app/package.json src/ui/app/package.json
RUN npm ci
COPY src/ui/ src/ui/
RUN npm run build:ui

FROM $LENS_BUILD_IMAGE AS builder
RUN apk add --no-cache rust build-base cmake perl pkgconf openssl-dev git
WORKDIR /src
COPY .cargo/ .cargo/
COPY src/worker/ src/worker/
COPY src/cli/ src/cli/
WORKDIR /src/src/worker
ARG TARGETARCH
ARG LENS_CARGO_JOBS=4
ENV CARGO_BUILD_JOBS=${LENS_CARGO_JOBS} CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols \
    CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
# Cargo rebuilds by mtime, which COPY keeps from the checkout, so a second checkout can look older than the cached
# build. Touching every file whose hash changed since the last build makes Cargo rebuild it.
RUN --mount=type=cache,id=lens-cargo-registry,target=/root/.cargo/registry \
    --mount=type=cache,id=lens-cargo-git,target=/root/.cargo/git \
    --mount=type=cache,id=lens-cargo-target-${TARGETARCH},target=/src/src/worker/target \
    find /src/.cargo /src/src -type f -not -path '/src/src/worker/target/*' -exec sha256sum {} + | sort > /tmp/source-hashes && \
    if [ -f target/source-hashes ]; then comm -13 target/source-hashes /tmp/source-hashes | cut -c67- | tr '\n' '\0' | xargs -0 -r touch; fi && \
    cp /tmp/source-hashes target/source-hashes && \
    cargo build --locked --release -p litellm-lens -p lens-migrate -p lens-agents --bins && \
    mkdir /out && cp target/release/litellm-lens target/release/lens-migrate target/release/lens-agent-runtime /out/
FROM ghcr.io/astral-sh/uv:0.13.9@sha256:d3b601f0c7b118bc1753febf7c41cc80981df678445e0254284eecf84db79a7d AS uv

FROM $LENS_RUNTIME_IMAGE AS native-deps
RUN apk add --no-cache python-3.14=3.14.10_git20261118-r0 python-3.14-dev=3.14.10_git20261118-r0 build-base git libffi-dev openssl-dev
COPY --from=uv /uv /usr/local/bin/uv
ENV UV_PYTHON_DOWNLOADS=never UV_LINK_MODE=copy
RUN git init /opt/hermes && git -C /opt/hermes remote add origin https://github.com/NousResearch/hermes-agent.git && \
    git -C /opt/hermes fetch --depth 1 origin tag v0.23.4 && \
    test "$(git -C /opt/hermes rev-parse 'v0.23.4^{commit}')" = 8fc0374447313e276f6b9a2764bfc750190c966a && \
    git -C /opt/hermes checkout --detach 8fc0374447313e276f6b9a2764bfc750190c966a
WORKDIR /opt/hermes
RUN hermes_version="$(git describe --tags --exact-match HEAD)" && hermes_version="${hermes_version#v}" && \
    sed -i "s/^version = \"0.0.0\"$/version = \"$hermes_version\"/" pyproject.toml uv.lock && \
    uv sync --frozen --no-dev --extra messaging --extra mcp --python /usr/bin/python3.14 && \
    .venv/bin/python scripts/write_install_stamp.py --output install-stamp.json \
      --distribution docker --update-mechanism external --source ci --base-version "$hermes_version" --distance 0
COPY deploy/runtime/deepagents.lock /tmp/deepagents.lock
WORKDIR /opt
RUN uv venv /opt/deepagents --python /usr/bin/python3.14 && \
    uv pip sync --no-config --require-hashes --python /opt/deepagents/bin/python /tmp/deepagents.lock

FROM $LENS_RUNTIME_IMAGE AS agents
RUN apk add --no-cache python-3.14=3.14.10_git20261118-r0 bash git curl libgcc libstdc++ openssl ca-certificates ca-certificates-bundle libatomic
COPY --from=native-deps /opt/hermes /opt/hermes
COPY --from=native-deps /opt/deepagents /opt/deepagents
COPY --from=builder /out/lens-agent-runtime /usr/local/bin/lens-agent-runtime
ENV LENS_DEEPAGENTS_EXECUTABLE=/opt/deepagents/bin/dcode LENS_HERMES_EXECUTABLE=/opt/hermes/.venv/bin/hermes \
    PYTHONDONTWRITEBYTECODE=1 LENS_RUNTIME_DIRECTORY=/var/lib/lens-runtime
EXPOSE 4320
HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=6 CMD curl --fail --silent http://127.0.0.1:4320/health > /dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/lens-agent-runtime"]

FROM $LENS_RUNTIME_IMAGE AS runtime
ARG LENS_VERSION=agent-first
RUN apk add --no-cache libgcc libstdc++ openssl ca-certificates ca-certificates-bundle curl libatomic && \
    mkdir -p /var/lib/lens && chown 65532:65532 /var/lib/lens
ENV LENS_VERSION=${LENS_VERSION} LENS_UI_DIRECTORY=/app/ui LENS_AGENT_DATABASE=/var/lib/lens/agents.db
WORKDIR /app
COPY --from=builder /out/litellm-lens /usr/local/bin/litellm-lens
COPY --from=builder /out/lens-migrate /usr/local/bin/lens-migrate
COPY --from=ui-builder /src/src/ui/app/out/ /app/ui/
USER 65532:65532
EXPOSE 4318
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=6 CMD curl --fail --silent http://127.0.0.1:4318/health/ready > /dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/litellm-lens"]
