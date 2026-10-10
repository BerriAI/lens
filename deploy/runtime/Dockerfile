ARG LENS_BUILD_IMAGE=cgr.dev/chainguard/wolfi-base@sha256:1d95114038f76513a9ace6fca107d5582b08c65981f81f61cb56bf7fd2ef216d
ARG LENS_RUNTIME_IMAGE=cgr.dev/chainguard/wolfi-base@sha256:1d95114038f76513a9ace6fca107d5582b08c65981f81f61cb56bf7fd2ef216d
ARG LENS_UI_BUILD_IMAGE=node:24.14.1-bookworm-slim@sha256:b506e7321f176aae77317f99d67a24b272c1f09f1d10f1761f2773447d8da26c
ARG LENS_SLACK_BUILD_IMAGE=node:24.20.0-bookworm-slim@sha256:ba849c60be29959425b8734d57b8b4b7d56f98edd9504c9af091d5281095a71e

FROM $LENS_UI_BUILD_IMAGE AS ui-builder
WORKDIR /src
ENV NEXT_TELEMETRY_DISABLED=1
COPY package.json package-lock.json ./
COPY src/ui/lib/package.json src/ui/lib/package.json
COPY src/ui/app/package.json src/ui/app/package.json
RUN npm ci
COPY src/ui/ src/ui/
RUN npm run build:ui

FROM $LENS_SLACK_BUILD_IMAGE AS slack-builder
WORKDIR /app/slack-agent
COPY src/slack-agent/package.json src/slack-agent/package-lock.json ./
RUN npm ci --ignore-scripts
COPY src/slack-agent/tsconfig.json ./
COPY src/slack-agent/src/ src/
COPY src/slack-agent/assets/ assets/
RUN npm run build && npm prune --omit=dev --ignore-scripts && rm -f dist/*.test.js

FROM $LENS_BUILD_IMAGE AS builder
RUN apk add --no-cache rust build-base cmake perl pkgconf openssl-dev libseccomp-dev python-3.13 jq git
WORKDIR /src
COPY .cargo/ .cargo/
COPY src/worker/ src/worker/
WORKDIR /src/src/worker
ENV CARGO_BUILD_JOBS=2 CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols \
    CARGO_PROFILE_RELEASE_LTO=thin CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
RUN cargo build --locked --release -p litellm-lens
RUN cargo build --locked --release -p lens-migrate
RUN cc -std=c11 -D_GNU_SOURCE -O2 -Wall -Wextra -Werror crates/lens/sandbox/python_policy.c -lseccomp -o /tmp/python-policy && \
    /tmp/python-policy /tmp/python.seccomp

FROM builder AS test-builder
RUN cargo test --locked --release -p litellm-lens --test sandbox --no-run --message-format=json > /tmp/test-artifacts.json && \
    jq -r 'select(.executable != null and .target.name == "sandbox") | .executable' /tmp/test-artifacts.json > /tmp/sandbox-artifact && \
    test "$(wc -l < /tmp/sandbox-artifact)" -eq 1 && \
    cp "$(cat /tmp/sandbox-artifact)" /tmp/lens-sandbox-tests && \
    chmod 755 /tmp/lens-sandbox-tests

FROM $LENS_RUNTIME_IMAGE AS service
ARG LENS_VERSION=0.1.0-dev.0
RUN apk add --no-cache python-3.13 setpriv libgcc libstdc++ openssl ca-certificates curl
ENV LENS_VERSION=${LENS_VERSION} PYTHONDONTWRITEBYTECODE=1 LENS_UI_DIRECTORY=/app/ui
WORKDIR /app
COPY --from=builder /src/src/worker/target/release/litellm-lens /usr/local/bin/litellm-lens
COPY --from=builder /src/src/worker/target/release/lens-migrate /usr/local/bin/lens-migrate
COPY --from=builder /tmp/python.seccomp /app/lens/python.seccomp
COPY --from=ui-builder /src/src/ui/app/out/ /app/ui/
COPY --from=slack-builder /usr/local/bin/node /usr/local/bin/node
COPY --from=slack-builder /app/slack-agent/ /app/slack-agent/
COPY src/worker/crates/lens/sandbox/python_runtime.py /tmp/python_runtime.py
RUN python3.13 -S /tmp/python_runtime.py /app/lens/python-runtime.json && rm /tmp/python_runtime.py
USER 65532:65532
EXPOSE 4318
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=6 CMD curl --fail --silent http://127.0.0.1:4318/health/ready > /dev/null || exit 1
ENTRYPOINT ["/usr/local/bin/node", "/app/slack-agent/dist/supervisor.js"]

FROM service AS smoke
COPY --from=test-builder /tmp/lens-sandbox-tests /usr/local/bin/lens-sandbox-tests
ENTRYPOINT ["/usr/local/bin/lens-sandbox-tests"]
CMD ["--ignored", "--nocapture", "--test-threads=1"]

FROM service AS runtime
