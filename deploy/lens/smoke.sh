#!/usr/bin/env bash
set -euo pipefail

lens_image="${1:?Pass the built Lens image reference}"
lens_release="${2:?Pass the expected Lens version}"
lens_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
lens_temp="$(mktemp -d)"
lens_name="lens-smoke-$$"
lens_storage="$lens_name-clickhouse"
cleanup() {
  lens_status=$?
  if [[ "$lens_status" -ne 0 ]]; then
    docker logs --tail 60 "$lens_name" >&2 || true
  fi
  docker rm -fv "$lens_name" "$lens_storage" >/dev/null 2>&1 || true
  docker network rm "$lens_name" >/dev/null 2>&1 || true
  rm -rf "$lens_temp"
}
trap cleanup EXIT

version="$(docker run --rm --network none --read-only --cap-drop ALL --security-opt no-new-privileges "$lens_image" --version)"
test "$version" = "litellm-lens $lens_release protocol=7"
docker run --rm --user "$(id -u):$(id -g)" --volume "$lens_temp:/setup" "$lens_image" init /setup
before="$(shasum -a 256 "$lens_temp/.env")"
docker run --rm --user "$(id -u):$(id -g)" --volume "$lens_temp:/setup" "$lens_image" init /setup
test "$before" = "$(shasum -a 256 "$lens_temp/.env")"
docker network create "$lens_name" >/dev/null
docker run -d --name "$lens_storage" --network "$lens_name" --env-file "$lens_temp/.env" \
  -e CLICKHOUSE_USER=lens -e CLICKHOUSE_DB=lens -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  --volume "$lens_root/deploy/clickhouse/keeper.xml:/etc/clickhouse-server/config.d/lens-keeper.xml:ro" \
  mirror.gcr.io/clickhouse/clickhouse-server:26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e >/dev/null
wait_storage() {
  for attempt in $(seq 1 60); do
    if docker exec "$lens_storage" sh -c 'clickhouse-client --host "$1" --user lens --password "$CLICKHOUSE_PASSWORD" --query "SELECT 1"' sh "$lens_storage" >/dev/null 2>&1; then
      return
    fi
    test "$attempt" -lt 60
    sleep 1
  done
}
wait_storage
docker run -d --name "$lens_name" --network "$lens_name" --read-only --cap-drop ALL \
  --security-opt no-new-privileges --pids-limit 128 --memory 2g --cpus 2 \
  --tmpfs /tmp:rw,noexec,nosuid,size=1g --env-file "$lens_temp/.env" \
  -e LENS_MODE=standalone -e CLICKHOUSE_HOST="$lens_storage" -e CLICKHOUSE_USER=lens \
  -e CLICKHOUSE_DATABASE=lens "$lens_image" >/dev/null
wait_ready() {
  for attempt in $(seq 1 60); do
    test "$(docker inspect --format '{{.State.Running}}' "$lens_name")" = true
    if docker exec "$lens_name" curl --fail --silent http://127.0.0.1:4318/health/ready >/dev/null 2>&1; then
      return
    fi
    test "$attempt" -lt 60
    sleep 1
  done
}
wait_ready
test "$(docker exec "$lens_name" id -u)" = 65532
docker exec "$lens_name" curl --fail --silent http://127.0.0.1:4318/ui/ > "$lens_temp/ui.html"
test -s "$lens_temp/ui.html"
docker exec "$lens_name" sh -c 'curl --fail --silent --header "Authorization: Bearer $LENS_ADMIN_TOKEN" http://127.0.0.1:4318/lens' > "$lens_temp/lenses.json"
test "$(jq '.lenses | length' "$lens_temp/lenses.json")" = 0
test "$(docker exec "$lens_name" curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:4318/lens)" = 401
docker exec "$lens_name" sh -c 'curl --fail --silent --header "Authorization: Bearer $LENS_ADMIN_TOKEN" --header "Content-Type: application/json" --data '\''{"name":"Container smoke tracing"}'\'' http://127.0.0.1:4318/lens/tracing/keys' > "$lens_temp/key.json"
jq -er '"Authorization: Bearer " + .key' "$lens_temp/key.json" > "$lens_temp/tracing-header"
docker exec -i "$lens_name" sh -c 'umask 077; cat > /tmp/lens-smoke-header' < "$lens_temp/tracing-header"
jq -n --arg now "$(date +%s)000000000" '{resourceSpans:[{resource:{attributes:[{key:"service.name",value:{stringValue:"container-smoke"}}]},scopeSpans:[{spans:[{traceId:"11112222333344445555666677778888",spanId:"1111222233334444",name:"Container smoke trace",startTimeUnixNano:$now,endTimeUnixNano:$now,attributes:[{key:"gen_ai.operation.name",value:{stringValue:"invoke_agent"}},{key:"gen_ai.agent.name",value:{stringValue:"container-smoke"}}],status:{code:1}}]}]}]}' > "$lens_temp/trace.json"
for attempt in $(seq 1 30); do
  if docker exec -i "$lens_name" curl --fail --silent --header @/tmp/lens-smoke-header --header 'Content-Type: application/json' --data-binary @- http://127.0.0.1:4318/v1/traces < "$lens_temp/trace.json" > /dev/null; then
    break
  fi
  test "$attempt" -lt 30
  sleep 1
done
docker exec "$lens_name" sh -c 'curl --fail --silent --header "Authorization: Bearer $LENS_ADMIN_TOKEN" http://127.0.0.1:4318/v1/traces' > "$lens_temp/traces.json"
test "$(jq -r '.data[0].trace_id' "$lens_temp/traces.json")" = 11112222333344445555666677778888
test "$(jq '.data | length' "$lens_temp/traces.json")" = 1
docker restart "$lens_storage" >/dev/null
wait_storage
docker restart "$lens_name" >/dev/null
wait_ready
docker exec "$lens_name" sh -c 'curl --fail --silent --header "Authorization: Bearer $LENS_ADMIN_TOKEN" http://127.0.0.1:4318/lens' > "$lens_temp/restarted.json"
test "$(jq '.lenses | length' "$lens_temp/restarted.json")" = 0
docker exec "$lens_name" sh -c 'curl --fail --silent --header "Authorization: Bearer $LENS_ADMIN_TOKEN" http://127.0.0.1:4318/v1/traces' > "$lens_temp/restarted-traces.json"
test "$(jq -r '.data[0].trace_id' "$lens_temp/restarted-traces.json")" = 11112222333344445555666677778888
test "$(jq '.data | length' "$lens_temp/restarted-traces.json")" = 1
printf '%s\n' 'Standalone Lens serves the UI, enforces API access, ingests a trace, and retains it after restarting both services'
