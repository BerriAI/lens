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
  [[ "${BUILD_FAIL:-false}" != true ]] || exit 1
  printf '%s\n' "${RUSTUP_TOOLCHAIN:-}:${MATURIN_NO_INSTALL_RUST:-}" >> "$RUNNER_TEMP/build-environment"
  printf '%s\n' "$*" >> "$RUNNER_TEMP/installed"
fi
EOF
cat > "$fixture/bin/rustup" <<'EOF'
#!/bin/bash
set -euo pipefail
printf '%s\n' "$*" >> "$RUNNER_TEMP/rustup"
[[ "${RUSTUP_FAIL:-false}" != true ]]
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
chmod +x "$fixture/bin/python" "$fixture/bin/gh" "$fixture/bin/rustup"
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

LENS_INSTALL_FROM_SOURCE=true INSTALLED=true bash "$installer" > "$fixture/output" 2>&1
test ! -e "$fixture/downloaded"
grep -Fx 'toolchain install 1.99.0 --profile minimal' "$fixture/rustup"
grep -Fx '1.99.0:1' "$fixture/build-environment"
sdk=$(cd "$(dirname "$installer")/.." && pwd)
grep -Fx -- "-m pip --python python install --config-settings build-args=--locked $sdk" "$fixture/installed"
rm "$fixture/installed" "$fixture/rustup"

if LENS_INSTALL_FROM_SOURCE=true RUSTUP_FAIL=true bash "$installer" > "$fixture/output" 2>&1; then
  echo 'Failed Rust installation was accepted'
  exit 1
fi
test ! -e "$fixture/installed"
grep -F 'Could not install Rust' "$fixture/output"

if LENS_INSTALL_FROM_SOURCE=true BUILD_FAIL=true bash "$installer" > "$fixture/output" 2>&1; then
  echo 'Failed SDK build was accepted'
  exit 1
fi
test ! -e "$fixture/installed"
grep -F 'Lens SDK source installation failed' "$fixture/output"
echo 'Verified release installation, checksum rejection, source installation, and build failures'
