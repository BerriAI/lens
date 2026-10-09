set -euo pipefail

version=0.1.0a3
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
