set -euo pipefail

version=0.1.0a3
if [[ "${LENS_INSTALL_FROM_SOURCE:-false}" == true ]]; then
  sdk=$(cd "$(dirname "$0")/.." && pwd)
  if [[ ! -f "$sdk/../worker/Cargo.lock" || ! -f "$sdk/pyproject.toml" ]]; then
    echo '::error::The Lens Action checkout is missing its SDK source or Cargo.lock. Use the Action from a complete, pinned Lens commit'
    exit 2
  fi
  if ! command -v rustup >/dev/null; then
    echo '::error::Source installation requires rustup. Use a GitHub-hosted runner or install rustup on the self-hosted runner'
    exit 2
  fi
  if ! rustup toolchain install 1.99.0 --profile minimal; then
    echo '::error::Could not install Rust 1.99.0 for the Lens SDK source build. Check the runner network and rustup configuration'
    exit 2
  fi
  if ! RUSTUP_TOOLCHAIN=1.99.0 MATURIN_NO_INSTALL_RUST=1 python -m pip --python "$LENS_PYTHON" install \
    --config-settings build-args=--locked "$sdk"; then
    echo '::error::Lens SDK source installation failed. Check the build output, Python 3.11+ environment, and access to pinned Cargo dependencies'
    exit 2
  fi
  exit 0
fi

if "$LENS_PYTHON" -c "from lens import _native; from importlib.metadata import version; assert version('lens-evals') == '$version'" 2>/dev/null; then
  exit 0
fi

case "${RUNNER_OS}:${RUNNER_ARCH}" in
  Linux:X64) platform='manylinux*x86_64' ;;
  macOS:ARM64) platform='macosx*arm64' ;;
  macOS:X64) platform='macosx*x86_64' ;;
  Windows:X64) platform='win_amd64' ;;
  *) echo '::error::No Lens SDK wheel for this runner. Supported: Linux x64, macOS arm64/x64, Windows x64, Python 3.11+'; exit 2 ;;
esac

download=$(mktemp -d "${RUNNER_TEMP}/lens-sdk.XXXXXX")
trap 'rm -rf "$download"' EXIT
if ! gh release download "lens-evals-v${version}" --repo BerriAI/lens \
  --pattern "lens_evals-${version}-cp311-abi3-${platform}.whl" \
  --pattern SHA256SUMS --dir "$download"; then
  echo '::error::Cannot download Lens SDK wheels. Set sdk-token to a token with contents:read access to BerriAI/lens, or preinstall the release wheel in the selected Python environment'
  exit 2
fi

wheels=("$download"/*.whl)
if [[ ${#wheels[@]} -ne 1 || ! -f "${wheels[0]}" ]]; then
  echo '::error::Expected exactly one compatible Lens SDK wheel'
  exit 2
fi
wheel=${wheels[0]}
awk -v file="${wheel##*/}" '$2 == file { print }' "$download/SHA256SUMS" > "$download/selected.sha256"
test -s "$download/selected.sha256"
if command -v sha256sum >/dev/null; then
  (cd "$download" && sha256sum --check selected.sha256)
else
  (cd "$download" && shasum -a 256 --check selected.sha256)
fi
python -m pip --python "$LENS_PYTHON" install --only-binary=:all: "$wheel"
