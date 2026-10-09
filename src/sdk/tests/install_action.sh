set -euo pipefail

installer=$(cd "$(dirname "$0")/../action" && pwd)/install.sh
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir "$fixture/bin"
cat > "$fixture/bin/python" <<'EOF'
#!/bin/bash
if [[ "$1" == -c ]]; then
  [[ "${INSTALLED:-false}" == true ]]
else
  printf '%s\n' "$*" >> "$RUNNER_TEMP/installed"
fi
EOF
cat > "$fixture/bin/gh" <<'EOF'
#!/bin/bash
set -euo pipefail
echo downloaded >> "$RUNNER_TEMP/downloaded"
destination=${!#}
wheel=lens_evals-0.1.0a3-cp311-abi3-manylinux_2_28_x86_64.whl
echo wheel > "$destination/$wheel"
(cd "$destination" && shasum -a 256 "$wheel" > SHA256SUMS)
if [[ "${TAMPER:-false}" == true ]]; then echo corrupt >> "$destination/$wheel"; fi
EOF
chmod +x "$fixture/bin/python" "$fixture/bin/gh"
export PATH="$fixture/bin:$PATH" RUNNER_TEMP="$fixture" LENS_PYTHON=python RUNNER_OS=Linux RUNNER_ARCH=X64

bash "$installer" > "$fixture/output" 2>&1
test -s "$fixture/installed"
test -s "$fixture/downloaded"
rm "$fixture/installed" "$fixture/downloaded"

if TAMPER=true bash "$installer" > "$fixture/output" 2>&1; then
  echo 'Corrupted wheel was accepted'
  exit 1
fi
test ! -e "$fixture/installed"
rm "$fixture/downloaded"

INSTALLED=true bash "$installer" > "$fixture/output" 2>&1
test ! -e "$fixture/installed"
test ! -e "$fixture/downloaded"
echo 'Verified wheel installation, checksum rejection, and installed-package reuse'
