# Releasing Ferrogate

Releases are tag-driven and built only by GitHub Actions ([`release.yml`](../.github/workflows/release.yml)).
Nothing is published from a laptop.

## Steps

1. Make sure `main` is green.
2. Set the version in three places, in one pull request:
   - `Cargo.toml` → `[workspace.package] version`
   - `charts/ferrogate/Chart.yaml` → `version` and `appVersion`
   - `CHANGELOG.md` → rename `## [Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD` and add a new empty
     `## [Unreleased]` above it.
3. After merging, tag the merge commit and push the tag:

   ```sh
   git tag -s vX.Y.Z -m "Ferrogate vX.Y.Z"
   git push origin vX.Y.Z
   ```

4. The workflow runs and creates a **draft** GitHub release. Check the assets and notes, then
   publish the draft.

The workflow refuses to run if the tag, `Cargo.toml` and `Chart.yaml` versions differ, or if
`CHANGELOG.md` has no section for the version.

## What is built

| Artifact | Where | Integrity |
| --- | --- | --- |
| Static binaries: linux amd64/arm64 (musl), macOS arm64 | Release assets (`.tar.gz`) | `SHA256SUMS`; GitHub build provenance (`gh attestation verify`) |
| Container image, linux/amd64 + linux/arm64, distroless static, non-root | `ghcr.io/stochasticquant/ferrogate:X.Y.Z` | cosign keyless signature; SBOM and SLSA provenance attestations |
| Helm chart | `oci://ghcr.io/stochasticquant/charts/ferrogate` | cosign signature |
| Install manifest with the image pinned by digest | Release asset `ferrogate-install.yaml` | `SHA256SUMS` |
| CRD | Release asset | `SHA256SUMS` |

The image contains exactly the linux binaries attached to the release. Before anything is
pushed, the workflow runs the end-to-end test on kind with the release binary and the Helm chart.

Versions `0.x` and versions with a suffix (`-rc.1`) are marked as pre-releases. The `X.Y`
image tag is only moved for stable versions.

## Verifying a release

```sh
cosign verify ghcr.io/stochasticquant/ferrogate@sha256:<digest> \
  --certificate-identity-regexp 'https://github.com/stochasticquant/rust-kubernetes-operator/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com

gh attestation verify ferrogate-X.Y.Z-linux-amd64.tar.gz --repo stochasticquant/rust-kubernetes-operator
sha256sum -c SHA256SUMS
```

## Dry runs

Pull requests that change the release workflow, `Dockerfile.release` or the chart run the
whole pipeline (binaries, e2e, multi-arch image, chart package) without publishing.
