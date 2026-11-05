FROM cgr.dev/chainguard/wolfi-base@sha256:1d95114038f76513a9ace6fca107d5582b08c65981f81f61cb56bf7fd2ef216d AS policy
RUN apk add --no-cache build-base libseccomp-dev
COPY src/worker/crates/lens/sandbox/python_policy.c /tmp/python_policy.c
RUN cc -std=c11 -D_GNU_SOURCE -O2 -Wall -Wextra -Werror /tmp/python_policy.c -lseccomp -o /tmp/python-policy && /tmp/python-policy /tmp/python.seccomp

FROM cgr.dev/chainguard/wolfi-base@sha256:1d95114038f76513a9ace6fca107d5582b08c65981f81f61cb56bf7fd2ef216d
RUN apk add --no-cache python-3.13 setpriv libgcc libstdc++ openssl ca-certificates
COPY --from=policy /tmp/python.seccomp /app/lens/python.seccomp
COPY src/worker/crates/lens/sandbox/python_runtime.py /tmp/python_runtime.py
RUN python3.13 -S /tmp/python_runtime.py /app/lens/python-runtime.json && rm /tmp/python_runtime.py
ENV LD_LIBRARY_PATH=/bench/onnxruntime/capi LENS_PYTHON_RUNTIME=/app/lens
WORKDIR /bench
