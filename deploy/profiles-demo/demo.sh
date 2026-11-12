#!/usr/bin/env bash
set -euo pipefail

here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd -- "$here/../.." && pwd)"
project="${LENS_DEMO_PROJECT:-lens-profiles-demo}"
lens_port="${LENS_PORT:-4320}"
litellm_port="${LITELLM_PORT:-4400}"
image="${LENS_IMAGE:-lens:profiles-demo}"
env_file="$here/.env"
launcher="bash deploy/profiles-demo/demo.sh"
export LENS_PORT="$lens_port" LITELLM_PORT="$litellm_port" LENS_PUBLIC_URL="http://localhost:$lens_port" LENS_IMAGE="$image"
compose=(docker compose --project-name "$project" --project-directory "$here"
  --file "$root/deploy/lens/compose.yaml" --file "$here/compose.litellm.yaml")

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
  (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null
}

random_hex() {
  od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
}

ensure_secret() {
  grep -q "^$1=." "$env_file" || printf '%s=%s\n' "$1" "$2" >>"$env_file"
}

ensure_env() {
  if [ ! -s "$env_file" ]; then
    docker run --rm --user "$(id -u):$(id -g)" --volume "$here:/setup" "$image" init /setup
  fi
  [ -n "$(tail -c1 "$env_file")" ] && printf '\n' >>"$env_file"
  ensure_secret LITELLM_MASTER_KEY "sk-$(random_hex)"
  ensure_secret LITELLM_LENS_SERVICE_TOKEN "$(random_hex)"
  ensure_secret LENS_GATEWAY_SECRET "$(random_hex)"
}

start() {
  for port in "$lens_port" "$litellm_port"; do
    [[ "$port" =~ ^[0-9]+$ ]] || fail "Ports must be numbers."
  done
  if [ -z "$("${compose[@]}" ps -q --status running 2>/dev/null)" ]; then
    port_busy "$lens_port" && fail "Port $lens_port is in use. Pick another, e.g. LENS_PORT=4330 $launcher"
    port_busy "$litellm_port" && fail "Port $litellm_port is in use. Pick another, e.g. LITELLM_PORT=4410 $launcher"
  fi
  if ! docker image inspect "$image" >/dev/null 2>&1; then
    printf 'Building %s from this checkout. The first build compiles the Lens Rust backend and UI, which takes a long time and several GB of memory.\n' "$image"
    docker build --file "$root/deploy/runtime/Dockerfile" --tag "$image" "$root"
  fi
  ensure_env
  printf 'Building the LiteLLM dashboard with this checkout'"'"'s Lens UI (Node only, a few minutes the first time).\n'
  "${compose[@]}" build litellm
  "${compose[@]}" up --detach --wait --wait-timeout 300
  "${compose[@]}" exec -T lens litellm-lens seed-profiles-demo
  cat <<TEXT

Lens task profiles sample data is ready.
  LiteLLM:   http://localhost:$litellm_port/ui  (open the Lens tab, then Profiles)
  Username:  admin
  Password:  run '$launcher password' to print it in your terminal
  Lens:      http://localhost:$lens_port/ui/?tab=profiles&agent=lens-demo-research  ('$launcher lens-token' prints its admin token)
Agents lens-demo-research (frozen sample tasks), lens-demo-coding (draft to review) and lens-demo-support (automatic groups).
Stop with '$launcher stop'. Data stays until '$launcher reset'.
TEXT
}

secret() {
  [ -s "$env_file" ] || fail "No credentials yet. Run '$launcher' first."
  sed -n "s/^$1=//p" "$env_file" | tail -n 1
}

case "${1:-start}" in
  start) require_docker; start ;;
  seed) require_docker; "${compose[@]}" exec -T lens litellm-lens seed-profiles-demo ;;
  password) secret LITELLM_MASTER_KEY ;;
  lens-token) secret LENS_ADMIN_TOKEN ;;
  stop) require_docker; "${compose[@]}" stop ;;
  reset) require_docker; "${compose[@]}" down --volumes ;;
  *) fail "Usage: $launcher [start|seed|password|lens-token|stop|reset]" ;;
esac
