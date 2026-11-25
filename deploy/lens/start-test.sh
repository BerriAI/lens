#!/usr/bin/env bash
set -euo pipefail
lens_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
lens_test_dir="$(mktemp -d)"
trap 'rm -rf "$lens_test_dir"' EXIT
mkdir -p "$lens_test_dir/bin" "$lens_test_dir/root/deploy/lens"
cp "$lens_dir/start" "$lens_test_dir/root/deploy/lens/start"
cat > "$lens_test_dir/bin/docker" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$STUB_LOG"
if [[ "$1 ${2:-}" == "image inspect" ]]; then test "${STUB_IMAGES:-missing}" == present; fi
STUB
chmod +x "$lens_test_dir/bin/docker"
run_start() {
  : > "$lens_test_dir/log"
  env -u LENS_IMAGE -u LENS_AGENTS_IMAGE "$@" PATH="$lens_test_dir/bin:$PATH" STUB_LOG="$lens_test_dir/log" \
    LITELLM_PROXY_API_BASE=https://gateway.example/v1 LITELLM_PROXY_API_KEY=fixture-key \
    "$lens_test_dir/root/deploy/lens/start" ${start_arguments:-} >/dev/null 2>&1
}
run_start
test "$(grep -c '^build ' "$lens_test_dir/log")" = 2
grep -q -- '--target agents' "$lens_test_dir/log"
grep -q 'up --detach --wait' "$lens_test_dir/log"
run_start STUB_IMAGES=present
! grep -q '^build ' "$lens_test_dir/log"
start_arguments=--build run_start STUB_IMAGES=present
test "$(grep -c '^build ' "$lens_test_dir/log")" = 2
run_start LENS_IMAGE=registry.example/lens:revision LENS_AGENTS_IMAGE=registry.example/lens-agents:revision
test "$(grep -c '^pull ' "$lens_test_dir/log")" = 2
! grep -q '^build ' "$lens_test_dir/log"
start_arguments=--build run_start LENS_IMAGE=registry.example/lens:revision LENS_AGENTS_IMAGE=registry.example/lens-agents:revision
test "$(grep -c '^build ' "$lens_test_dir/log")" = 2
! grep -q '^pull ' "$lens_test_dir/log"
if run_start LENS_IMAGE=registry.example/lens:revision; then exit 1; fi
if start_arguments=--unknown run_start; then exit 1; fi
for lens_config in keeper logging; do
  diff <(awk -v name="  $lens_config:" '/^  [a-z]+:$/ { inside = ($0 == name); next } inside && !/^    content: \|$/ { sub(/^      /, ""); print }' "$lens_dir/compose.yaml") "$lens_dir/../clickhouse/$lens_config.xml"
done
printf '%s\n' 'Candidate image selection passed'
