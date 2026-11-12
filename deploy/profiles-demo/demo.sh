#!/usr/bin/env bash
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../.." && pwd)"
project="${LENS_DEMO_PROJECT:-lens-profiles-demo}"
port="${LENS_PORT:-4320}"
image="${LENS_IMAGE:-lens:profiles-demo}"
launcher="bash deploy/profiles-demo/demo.sh"
export LENS_PORT="$port" LENS_PUBLIC_URL="http://localhost:$port" LENS_IMAGE="$image"
compose=(docker compose --project-name "$project" --project-directory "$here" --file "$root/deploy/lens/compose.yaml")

fail() {
  printf 'demo.sh: %s\n' "$*" >&2
  exit 1
}

require_docker() {
  command -v docker >/dev/null || fail "Docker is not installed."
  docker compose version >/dev/null 2>&1 || fail "Docker Compose v2 is missing."
  docker info >/dev/null 2>&1 || fail "Docker is not running."
}

port_busy() {
  (exec 3<>"/dev/tcp/127.0.0.1/$port") 2>/dev/null
}

build_image() {
  docker image inspect "$image" >/dev/null 2>&1 && return
  printf 'Building %s from this checkout. The first build compiles the Lens Rust backend and the UI, which takes a long time and several GB of memory; later runs reuse the image.\n' "$image"
  docker build --file "$root/deploy/runtime/Dockerfile" --tag "$image" "$root"
}

start() {
  [[ "$port" =~ ^[0-9]+$ ]] || fail "LENS_PORT must be a port number."
  if [ -z "$("${compose[@]}" ps -q --status running lens 2>/dev/null)" ] && port_busy; then
    fail "Port $port is in use. Pick another, e.g. LENS_PORT=4330 $launcher"
  fi
  build_image
  if [ ! -s "$here/.env" ]; then
    docker run --rm --user "$(id -u):$(id -g)" --volume "$here:/setup" "$image" init /setup
  fi
  "${compose[@]}" up --detach --wait --wait-timeout 180
  "${compose[@]}" exec -T lens litellm-lens seed-profiles-demo
  cat <<TEXT

Lens task profiles sample data is ready.
  URL:       http://localhost:$port/ui/?tab=profiles&agent=lens-demo-research
  Password:  run '$launcher password' to print the admin token in your terminal
Agents lens-demo-research (frozen sample tasks), lens-demo-coding (draft to review) and lens-demo-support (automatic groups).
Stop with '$launcher stop'. Data stays until '$launcher reset'.
TEXT
}

case "${1:-start}" in
  start) require_docker; start ;;
  seed) require_docker; "${compose[@]}" exec -T lens litellm-lens seed-profiles-demo ;;
  password)
    [ -s "$here/.env" ] || fail "No credentials yet. Run '$launcher' first."
    sed -n 's/^LENS_ADMIN_TOKEN=//p' "$here/.env" | tail -n 1
    ;;
  stop) require_docker; "${compose[@]}" stop ;;
  reset) require_docker; "${compose[@]}" down --volumes ;;
  *) fail "Usage: $launcher [start|seed|password|stop|reset]" ;;
esac
