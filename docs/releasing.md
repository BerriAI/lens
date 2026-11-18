# Release Lens

Lens releases independently of LiteLLM. Use the same stable version in `VERSION`, the runtime and Compose build defaults, the Helm chart and its test, and the two UI workspaces and npm lockfile. The Python eval SDK has its own version and is outside this release

After the release PR merges, run the [Lens release workflow](https://github.com/BerriAI/lens/actions/workflows/lens-release.yml) on `main` without a tag to rehearse it. This builds and smoke-tests the image on native AMD64 and ARM64 and retains the chart, UI package and Compose bundle with a placeholder image digest. The rehearsal does not publish to GHCR or GitHub Releases

```sh
gh workflow run lens-release.yml --repo BerriAI/lens --ref main
```

Once the rehearsal and CI are green, get explicit approval to publish. Confirm `BerriAI/lens` has write access to any existing GHCR packages named `lens` and `charts/lens`. Ensure the intended users have pull access to both, or make both public if the approved release is public. No cross-repo token or signing secret is needed

Only after that approval, tag the exact checked commit on `main` and push the tag. For the first release:

```sh
git fetch origin main
git tag -a v0.1.0 <approved-main-commit-sha> -m 'Lens 0.1.0'
git push origin refs/tags/v0.1.0
```

The tag push publishes `ghcr.io/berriai/lens:0.1.0` and `oci://ghcr.io/berriai/charts/lens` version `0.1.0`. The GitHub release attaches the chart, Compose bundle and `@litellm/lens-ui` tarball along with a signed manifest. Nothing is published to npm or PyPI. If the workflow stops after a registry write, check both registries before retrying the failed jobs on the same run

## Verify a release

Download the assets for the exact release. Verify the manifest with Cosign from the directory containing the downloaded files, substituting the release version if needed:

```sh
cosign verify-blob --bundle manifest.sigstore.json \
  --certificate-identity 'https://github.com/BerriAI/lens/.github/workflows/lens-release.yml@refs/tags/v0.1.0' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' manifest.txt
sed -n '/^[a-f0-9]\{64\}  /p' manifest.txt | sha256sum --check
```

The manifest identifies the source SHA and the exact image and chart digests. Both OCI artifacts also have Cosign signatures from the same workflow identity. Use the digest pins in the published chart and Compose bundle when deploying
