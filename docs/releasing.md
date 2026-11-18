# Release Lens

Lens releases independently of LiteLLM. Use the same stable version in `VERSION`, the runtime and Compose build defaults, the Helm chart and its test, and the two UI workspaces and npm lockfile. The Python eval SDK has its own version and is outside this release

Releases are built and published by the private [BerriAI/lens-releaser](https://github.com/BerriAI/lens-releaser) repository, and nothing in this repository can publish. Its `release.yml` workflow takes a tag and a commit on `main` here, and only accepts a commit whose `VERSION` matches the tag

After the release PR merges, rehearse the release. Manual runs default to a dry run, which builds and smoke-tests the image on native AMD64 and ARM64 and packages the chart, UI package and Compose bundle with a placeholder image digest. It does not tag, push to GHCR or create a GitHub release

```sh
gh workflow run release.yml --repo BerriAI/lens-releaser -f tag=v0.1.0 -f sha=<main-commit-sha>
```

Once the rehearsal and CI are green, run it again with `dry_run=false`. That run waits for the release approver on the `production-release` environment before it builds anything

```sh
gh workflow run release.yml --repo BerriAI/lens-releaser -f tag=v0.1.0 -f sha=<main-commit-sha> -f dry_run=false
```

The approved run publishes `ghcr.io/berriai/lens:0.1.0` and `oci://ghcr.io/berriai/charts/lens` version `0.1.0`. It attaches the chart, Compose bundle and `@litellm/lens-ui` tarball along with a signed manifest to a release in BerriAI/lens-releaser. Nothing is published to npm or PyPI

The pipeline does not write to this repository. Creating the public tag and GitHub release here is a manual last step for someone with write access, using the two commands the run prints in its summary

```sh
gh release download v0.1.0 -R BerriAI/lens-releaser -D lens-0.1.0
gh release create v0.1.0 -R BerriAI/lens --target <main-commit-sha> --title 'Lens 0.1.0' --generate-notes lens-0.1.0/*
```

## Verify a release

Download the assets for the exact release. Verify the manifest with Cosign from the directory containing the downloaded files:

```sh
cosign verify-blob --bundle manifest.sigstore.json \
  --certificate-identity 'https://github.com/BerriAI/lens-releaser/.github/workflows/release.yml@refs/heads/main' \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' manifest.txt
sed -n '/^[a-f0-9]\{64\}  /p' manifest.txt | sha256sum --check
```

The manifest identifies the source SHA and the exact image and chart digests. Both OCI artifacts also have Cosign signatures from the same workflow identity. Use the digest pins in the published chart and Compose bundle when deploying
