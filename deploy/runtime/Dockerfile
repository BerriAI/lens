ARG LENS_BUILD_IMAGE=cgr.dev/chainguard/wolfi-base@sha256:ae9575b121f5776e2f04e192064bc2c5665af9d1c425d620dc773aca31698712
ARG LENS_RUNTIME_IMAGE=cgr.dev/chainguard/wolfi-base@sha256:ae9575b121f5776e2f04e192064bc2c5665af9d1c425d620dc773aca31698712
ARG LENS_UI_BUILD_IMAGE=node:24.14.1-bookworm-slim@sha256:b506e7321f176aae77317f99d67a24b272c1f09f1d10f1761f2773447d8da26c
ARG LENS_AGENT_BUILD_IMAGE=node:24.22.3-bookworm-slim@sha256:dc26ae92c9fb15cf6b1d63e20f0944b2efe0f115b20e00be48fb23fceb061c54

FROM $LENS_UI_BUILD_IMAGE AS ui-builder
WORKDIR /src
ENV NEXT_TELEMETRY_DISABLED=1
COPY package.json package-lock.json ./
COPY src/ui/lib/package.json src/ui/lib/package.json
COPY src/ui/app/package.json src/ui/app/package.json
RUN npm ci
COPY src/ui/ src/ui/
RUN npm run build:ui

FROM $LENS_AGENT_BUILD_IMAGE AS agent-builder
WORKDIR /app/agent
COPY src/agent/package.json src/agent/package-lock.json ./
RUN npm ci --ignore-scripts
COPY src/agent/ ./
RUN npm run build
WORKDIR /app/agent-chat
COPY src/agent-chat/package.json src/agent-chat/package-lock.json ./
RUN npm ci --ignore-scripts
COPY src/agent-chat/ ./
RUN npm run build && npm prune --omit=dev --ignore-scripts && npm prune --prefix ../agent --omit=dev --ignore-scripts && \
    rm -f dist/*.test.js dist/*.test.d.ts dist/connectors/slack/*.test.js dist/connectors/slack/*.test.d.ts ../agent/dist/*.test.js ../agent/dist/*.test.d.ts

FROM $LENS_BUILD_IMAGE AS builder
RUN apk add --no-cache rust build-base cmake perl pkgconf openssl-dev libseccomp-dev python-3.13 jq git
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
    cargo build --locked --release -p litellm-lens && \
    cargo build --locked --release -p lens-migrate && \
    mkdir /out && cp target/release/litellm-lens target/release/lens-migrate /out/
RUN cc -std=c11 -D_GNU_SOURCE -O2 -Wall -Wextra -Werror crates/lens/sandbox/python_policy.c -lseccomp -o /tmp/python-policy && \
    /tmp/python-policy /tmp/python.seccomp

FROM builder AS test-builder
RUN --mount=type=cache,id=lens-cargo-registry,target=/root/.cargo/registry \
    --mount=type=cache,id=lens-cargo-git,target=/root/.cargo/git \
    --mount=type=cache,id=lens-cargo-target-${TARGETARCH},target=/src/src/worker/target \
    cargo test --locked --release -p litellm-lens --test sandbox --no-run --message-format=json > /tmp/test-artifacts.json && \
    jq -r 'select(.executable != null and .target.name == "sandbox") | .executable' /tmp/test-artifacts.json > /tmp/sandbox-artifact && \
    test "$(wc -l < /tmp/sandbox-artifact)" -eq 1 && \
    cp "$(cat /tmp/sandbox-artifact)" /tmp/lens-sandbox-tests && \
    chmod 755 /tmp/lens-sandbox-tests

FROM $LENS_RUNTIME_IMAGE AS service
ARG LENS_VERSION=0.6.0
RUN apk add --no-cache --upgrade python-3.13 setpriv libgcc libstdc++ openssl ca-certificates ca-certificates-bundle curl libatomic
ENV LENS_VERSION=${LENS_VERSION} PYTHONDONTWRITEBYTECODE=1 LENS_UI_DIRECTORY=/app/ui
WORKDIR /app
COPY --from=builder /out/litellm-lens /usr/local/bin/litellm-lens
COPY --from=builder /out/lens-migrate /usr/local/bin/lens-migrate
COPY --from=builder /tmp/python.seccomp /app/lens/python.seccomp
COPY --from=ui-builder /src/src/ui/app/out/ /app/ui/
COPY --from=agent-builder /usr/local/bin/node /usr/local/bin/node
COPY --from=agent-builder /app/agent/ /app/agent/
COPY --from=agent-builder /app/agent-chat/ /app/agent-chat/
COPY src/worker/crates/lens/sandbox/python_runtime.py /tmp/python_runtime.py
RUN python3.13 -S /tmp/python_runtime.py /app/lens/python-runtime.json && rm /tmp/python_runtime.py
USER 65532:65532
EXPOSE 4318
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=6 CMD curl --fail --silent http://127.0.0.1:4318/health/ready > /dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/node", "/app/agent-chat/dist/supervisor.js"]

FROM service AS smoke
COPY --from=test-builder /tmp/lens-sandbox-tests /usr/local/bin/lens-sandbox-tests
ENTRYPOINT ["/usr/local/bin/lens-sandbox-tests"]
CMD ["--ignored", "--nocapture", "--test-threads=1"]

FROM service AS runtime
