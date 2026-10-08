#!/usr/bin/env bash
set -euo pipefail

ref="${1:-origin/main}"
base=$(git merge-base "$ref" HEAD)
route_file="src/worker/crates/server/python_routes.txt"
repo_root=$(git rev-parse --show-toplevel)

base_route_file=""
for candidate in "$route_file" "runtime/crates/server/python_routes.txt"; do
    if git cat-file -e "$base:$candidate" 2>/dev/null; then
        base_route_file="$candidate"
        break
    fi
done

if [[ -z "$base_route_file" ]]; then
    printf 'No Python route inventory at base %s; ratchet passes\n' "$base"
    exit 0
fi

base_count=$(git show "$base:$base_route_file" | wc -l)
current_count=$(wc -l < "$repo_root/$route_file")

if (( current_count > base_count )); then
    printf 'Python route inventory grew from %s to %s lines\n' "$base_count" "$current_count" >&2
    exit 1
fi

printf 'Python route inventory did not grow: %s lines at base, %s now\n' "$base_count" "$current_count"
