#!/usr/bin/env bash
set -euo pipefail

version=$(<VERSION)
destination=${1:?Expected an output directory}
digest=${2:?Expected an image index digest}
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]
[[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]
image="ghcr.io/berriai/lens@$digest"

mkdir -p "$destination"
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
cp -R helm/lens "$stage/chart"
sed -i "s/^  digest: \"\"$/  digest: \"$digest\"/" "$stage/chart/values.yaml"
helm lint --strict "$stage/chart"
helm show chart "$stage/chart" | awk -v version="$version" '
  $1 == "version:" {chart = $2}
  $1 == "appVersion:" {app = $2; gsub(/"/, "", app)}
  END {exit !(chart == version && app == version)}
'
helm package "$stage/chart" --destination "$destination"
helm template lens "$destination/lens-$version.tgz" > "$stage/rendered.yaml"
grep -Fq "$image" "$stage/rendered.yaml"

npm pack --workspace @litellm/lens-ui --pack-destination "$destination" --ignore-scripts
test -s "$destination/litellm-lens-ui-$version.tgz"

bundle="$stage/lens-$version"
mkdir -p "$bundle/deploy/lens" "$bundle/deploy/clickhouse"
cp deploy/lens/{compose.yaml,start,backup,restore} "$bundle/deploy/lens/"
cp deploy/clickhouse/keeper.xml "$bundle/deploy/clickhouse/"
cp VERSION "$bundle/"
printf '%s\n' '#!/usr/bin/env bash' 'set -euo pipefail' \
  'exec "$(dirname -- "${BASH_SOURCE[0]}")/deploy/lens/start" "$@"' > "$bundle/start"
chmod +x "$bundle/start"
printf '%s\n' "$image" > "$bundle/deploy/lens/image"
LENS_IMAGE="$image" LENS_ADMIN_TOKEN=release-check CLICKHOUSE_PASSWORD=release-check \
  docker compose -f "$bundle/deploy/lens/compose.yaml" config --no-env-resolution --images lens | grep -Fx "$image"
tar -czf "$destination/lens-install-$version.tar.gz" -C "$stage" "lens-$version"
