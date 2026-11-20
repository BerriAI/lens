#!/usr/bin/env bash
set -euo pipefail
lens_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/bin"
cat > "$work/bin/docker" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$STUB_LOG"
setup_dir="$STUB_ROOT/deploy/lens"
case "$1 ${2:-}" in
  "image inspect") exit 1 ;;
  "pull "*) [[ "${STUB_PULL:-ok}" == ok ]] ;;
  "run "*) [[ -f "$setup_dir/.env" ]] || printf 'LENS_ADMIN_TOKEN=stub\n' > "$setup_dir/.env" ;;
  "compose "*)
    case "$*" in
      *"config --images"*) sed -n 's/^LENS_IMAGE=//p' "$setup_dir/.env" | grep . || printf 'ghcr.io/berriai/lens:1.2.3\n' ;;
      *" up "*) printf '%s\n' "up ${LENS_IMAGE:-$(sed -n 's/^LENS_IMAGE=//p' "$setup_dir/.env")}" >> "$STUB_LOG" ;;
      *" exec "*) printf 'http://localhost:4318' ;;
    esac ;;
esac
STUB
chmod +x "$work/bin/docker"
failures=0
new_checkout() {
  rm -rf "$work/root"
  mkdir -p "$work/root/deploy/lens"
  cp "$lens_dir/start" "$work/root/deploy/lens/start"
  printf '1.2.3\n' > "$work/root/VERSION"
  : > "$work/log"
}
start() {
  env -u LENS_IMAGE "$@" PATH="$work/bin:$PATH" STUB_LOG="$work/log" STUB_ROOT="$work/root" \
    "$work/root/deploy/lens/start" ${start_arguments:-} >/dev/null 2>&1
}
expect() {
  local name="$1" started="$2" built="$3"
  local actual_built=no
  grep -q '^build ' "$work/log" && actual_built=yes
  if [[ "$(grep '^up ' "$work/log" | tail -n 1)" != "up $started" || "$actual_built" != "$built" ]]; then
    printf 'FAIL %s\n' "$name" >&2
    sed 's/^/  /' "$work/log" >&2
    failures=$((failures + 1))
  fi
  : > "$work/log"
}
published=ghcr.io/berriai/lens:1.2.3

new_checkout
start
expect 'a fresh checkout runs the published image without building' "$published" no
start
expect 'a restart keeps the published image without building' "$published" no
start_arguments=--build start
expect 'the build flag builds the checkout' lens:local yes

new_checkout
start_arguments=--build start
expect 'a fresh checkout with the build flag builds the checkout' lens:local yes
start
expect 'a source installation keeps building on restart' lens:local yes

new_checkout
start STUB_PULL=fail
expect 'a failed pull falls back to a source build' lens:local yes

new_checkout
printf 'LENS_ADMIN_TOKEN=stub\n' > "$work/root/deploy/lens/.env"
start
expect 'an installation without a recorded image starts the compose default without building' '' no

new_checkout
start LENS_IMAGE=example.test/lens:custom
expect 'an explicit image wins' example.test/lens:custom no

new_checkout
printf 'example.test/lens@sha256:abc\n' > "$work/root/deploy/lens/image"
start
expect 'a release bundle image file wins' example.test/lens@sha256:abc no

new_checkout
if start_arguments=--unknown start; then
  printf 'FAIL an unknown argument is rejected\n' >&2
  failures=$((failures + 1))
fi

compose_default="$(sed -n 's/^    image: ${LENS_IMAGE:-\(.*\)}$/\1/p' "$lens_dir/compose.yaml" | sort -u)"
if [[ "$compose_default" != "ghcr.io/berriai/lens:$(tr -d '[:space:]' < "$lens_dir/../../VERSION")" ]]; then
  printf 'FAIL the compose default image is the release in VERSION, found %s\n' "$compose_default" >&2
  failures=$((failures + 1))
fi

if ! diff <(sed -n '/^    content: |$/,$p' "$lens_dir/compose.yaml" | sed '1d; s/^      //') "$lens_dir/../clickhouse/keeper.xml" >&2; then
  printf 'FAIL the Keeper configuration in compose.yaml matches deploy/clickhouse/keeper.xml\n' >&2
  failures=$((failures + 1))
fi

[[ "$failures" == 0 ]]
printf 'start image selection passed\n'
