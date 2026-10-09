#!/usr/bin/env bash
set -euo pipefail

lens_mode="${1:?baseline or candidate}"
lens_runtime="${2:?immutable runtime image}"
lens_report="${3:?report directory}"
lens_source_sha="${4:?source commit}"
lens_gateway_image="${5:-}"
lens_runner=/tmp/lens-feedback-qualified.YYruui/target/debug/service-benchmark
lens_root=/Users/mfkhalil/Code/litellm-lens
lens_name="lens-paired-$lens_mode-$$"
lens_private="$(mktemp -d /tmp/lens-paired-private.XXXXXX)"
mkdir -p "$lens_report"
chmod 700 "$lens_private"
lens_admin="sk-benchmark-$(openssl rand -hex 24)"
lens_service="$(openssl rand -hex 24)"
lens_password="$(openssl rand -hex 24)"
lens_runtime_name="$lens_name-runtime"
lens_gateway="$lens_name-gateway"
lens_postgres="$lens_name-postgres"
lens_clickhouse="$lens_name-clickhouse"
lens_read_port=14320
lens_ingest_port=14319
if [[ "$lens_mode" == candidate ]]; then
  lens_read_port=14321
  lens_ingest_port=14321
elif [[ "$lens_mode" != baseline || -z "$lens_gateway_image" ]]; then
  exit 2
fi
cleanup() {
  for name in "$lens_runtime_name" "$lens_gateway" "$lens_postgres" "$lens_clickhouse"; do
    docker logs "$name" > "$lens_private/$name.log" 2>&1 || true
    docker rm -fv "$name" >/dev/null 2>&1 || true
  done
  docker network rm "$lens_name" >/dev/null 2>&1 || true
  printf '%s\n' "$lens_private" > "$lens_report/$lens_mode-private-log-path"
  rm -f "$lens_private/config.json" "$lens_private/key.json" "$lens_private/admin.curl" "$lens_private/tracing.curl"
}
trap cleanup EXIT
printf 'header = "Authorization: Bearer %s"\n' "$lens_admin" > "$lens_private/admin.curl"
chmod 600 "$lens_private/admin.curl"
docker network create "$lens_name" >/dev/null
docker create --name "$lens_clickhouse" --network "$lens_name" --network-alias clickhouse --memory 2g --cpus 2 \
  -e CLICKHOUSE_USER=lens -e CLICKHOUSE_DB=lens -e CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
  -e "CLICKHOUSE_PASSWORD=$lens_password" \
  clickhouse/clickhouse-server:26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e >/dev/null
docker cp "$lens_root/deploy/clickhouse/keeper.xml" "$lens_clickhouse:/etc/clickhouse-server/config.d/lens-keeper.xml"
docker start "$lens_clickhouse" >/dev/null
for attempt in $(seq 1 90); do
  if docker exec "$lens_clickhouse" clickhouse-client --host clickhouse --user lens --password "$lens_password" --query 'SELECT 1' >/dev/null 2>&1; then break; fi
  test "$attempt" -lt 90
  sleep 1
done
if [[ "$lens_mode" == baseline ]]; then
  docker run -d --name "$lens_postgres" --network "$lens_name" --network-alias postgres --memory 512m --cpus .5 \
    -e "POSTGRES_PASSWORD=$lens_password" \
    postgres:16-alpine@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea >/dev/null
  for attempt in $(seq 1 60); do
    if docker exec "$lens_postgres" pg_isready -U postgres >/dev/null 2>&1; then break; fi
    test "$attempt" -lt 60
    sleep 1
  done
  docker run --rm --network "$lens_name" --memory 1g --cpus 1 --entrypoint /app/.venv/bin/prisma \
    -e "DATABASE_URL=postgres://postgres:$lens_password@postgres:5432/postgres" "$lens_gateway_image" \
    db push --schema /baseline/schema.prisma --skip-generate > "$lens_private/prisma-push.log" 2>&1
  cat > "$lens_private/gateway.yaml" <<'YAML'
model_list: []
general_settings:
  master_key: os.environ/LITELLM_MASTER_KEY
  database_url: os.environ/DATABASE_URL
  disable_prisma_schema_update: true
  disable_model_info_refresh: true
  database_connection_pool_limit: 5
YAML
  docker run -d --name "$lens_gateway" --network "$lens_name" --network-alias gateway --memory 1536m --cpus 1 \
    --cap-drop ALL --security-opt no-new-privileges --pids-limit 256 \
    -p "127.0.0.1:$lens_read_port:4000" \
    --mount "type=bind,src=$lens_private/gateway.yaml,dst=/baseline/benchmark.yaml,readonly" \
    -e "DATABASE_URL=postgres://postgres:$lens_password@postgres:5432/postgres" \
    -e "LITELLM_MASTER_KEY=$lens_admin" -e "LITELLM_LENS_SERVICE_TOKEN=$lens_service" \
    -e LITELLM_LENS_URL=http://lens:4318 -e PYTHONDONTWRITEBYTECODE=1 \
    "$lens_gateway_image" --config /baseline/benchmark.yaml --port 4000 --host 0.0.0.0 --num_workers 1 >/dev/null
  docker run -d --name "$lens_runtime_name" --network "$lens_name" --network-alias lens --memory 512m --cpus 1 \
    --read-only --cap-drop ALL --security-opt no-new-privileges --pids-limit 128 --tmpfs /tmp:rw,noexec,nosuid,size=128m \
    -p "127.0.0.1:$lens_ingest_port:4318" \
    -e LITELLM_URL=http://gateway:4000/ -e "LITELLM_LENS_SERVICE_TOKEN=$lens_service" \
    -e CLICKHOUSE_HOST=clickhouse -e CLICKHOUSE_USER=lens -e CLICKHOUSE_DATABASE=lens -e "CLICKHOUSE_PASSWORD=$lens_password" \
    "$lens_runtime" >/dev/null
else
  docker run -d --name "$lens_runtime_name" --network "$lens_name" --network-alias lens --memory 2g --cpus 2 \
    --read-only --cap-drop ALL --security-opt no-new-privileges --pids-limit 128 --tmpfs /tmp:rw,noexec,nosuid,size=128m \
    -p "127.0.0.1:$lens_read_port:4318" \
    -e LENS_MODE=standalone -e "LENS_ADMIN_TOKEN=$lens_admin" \
    -e CLICKHOUSE_HOST=clickhouse -e CLICKHOUSE_USER=lens -e CLICKHOUSE_DATABASE=lens -e "CLICKHOUSE_PASSWORD=$lens_password" \
    "$lens_runtime" >/dev/null
fi
for attempt in $(seq 1 120); do
  if curl --fail --silent --max-time 2 "http://127.0.0.1:$lens_ingest_port/health/ready" >/dev/null; then break; fi
  test "$attempt" -lt 120
  sleep 1
done
curl --fail --silent --show-error --max-time 30 --config "$lens_private/admin.curl" \
  -H 'Content-Type: application/json' --data '{"name":"Isolated paired benchmark"}' \
  "http://127.0.0.1:$lens_read_port/lens/tracing/keys" > "$lens_private/key.json"
jq -r '"header = \"Authorization: Bearer " + .key + "\""' "$lens_private/key.json" > "$lens_private/tracing.curl"
chmod 600 "$lens_private/tracing.curl"
for attempt in $(seq 1 45); do
  if curl --fail --silent --max-time 2 --config "$lens_private/tracing.curl" \
    -H 'Content-Type: application/json' --data '{"trace_id":"00000000000000000000000000000000","span_ids":[]}' \
    "http://127.0.0.1:$lens_ingest_port/v1/traces/receipt" > /dev/null; then break; fi
  test "$attempt" -lt 45
  sleep 1
done
lens_runtime_id="$(docker image inspect "$lens_runtime" --format '{{.Id}}')"
lens_artifact="$lens_runtime_id"
if [[ "$lens_mode" == baseline ]]; then
  lens_gateway_id="$(docker image inspect "$lens_gateway_image" --format '{{.Id}}')"
  lens_artifact="runtime=$lens_runtime_id;gateway=$lens_gateway_id"
fi
if [[ ! -f "$lens_report/timestamp-ms" ]]; then node -p 'Date.now() - 3600000' > "$lens_report/timestamp-ms"; fi
jq -n --slurpfile key "$lens_private/key.json" --arg admin "$lens_admin" \
  --arg base "http://127.0.0.1:$lens_read_port/" --arg ingestion "http://127.0.0.1:$lens_ingest_port/" \
  --arg source "$lens_source_sha" --arg artifact "$lens_artifact" --argjson stamp "$(cat "$lens_report/timestamp-ms")" \
  '{base_url:$base,ingestion_url:$ingestion,admin_token:$admin,tracing_key:$key[0].key,source_sha:$source,artifact:$artifact,timestamp_ms:$stamp,
    resources:"Linux arm64 on same Docker Desktop host; total application cap 2 CPU/2 GiB (baseline gateway 1 CPU/1536 MiB plus runtime 1 CPU/512 MiB; candidate runtime 2 CPU/2 GiB); each ClickHouse 2 CPU/2 GiB, image eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e, separate fresh Docker storage and same Keeper configuration; baseline additionally requires PostgreSQL16 0.5 CPU/512 MiB for metadata; runs sequentially"}' \
  > "$lens_private/config.json"
chmod 600 "$lens_private/config.json"
docker inspect "$lens_runtime_name" "$lens_clickhouse" | jq '[.[] | {Name,Image,platform:.Platform,resources:(.HostConfig|{Memory,NanoCpus,PidsLimit,ReadonlyRootfs})}]' > "$lens_report/$lens_mode-containers.json"
if [[ "$lens_mode" == baseline ]]; then
  docker inspect "$lens_gateway" "$lens_postgres" | jq '[.[] | {Name,Image,platform:.Platform,resources:(.HostConfig|{Memory,NanoCpus,PidsLimit,ReadonlyRootfs})}]' > "$lens_report/baseline-support-containers.json"
fi
sleep 5
"$lens_runner" run "$lens_private/config.json" > "$lens_report/$lens_mode.json"
printf '%s\n' "$lens_mode benchmark completed with verified telemetry readback"
