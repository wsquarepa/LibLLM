# Releasing

Stable releases are tag-driven. A push to `master` runs tests and clippy only; it does not produce a release. To cut a stable release:

1. Bump `workspace.package.version` in `Cargo.toml` and merge the bump into `master` (`chore(release): bump workspace version to X.Y.Z`).
2. After the bump lands, push a matching tag: `git tag vX.Y.Z && git push origin vX.Y.Z`. The `v` prefix is required.

CI rejects mismatches between the tag (`vX.Y.Z`) and the Cargo workspace version (`X.Y.Z`). "Bump version" and "cut a release" both need both steps; bumping alone produces nothing.

Backports are handled automatically: if `vX.Y.Z` is older than the highest existing v-tag at push time, CI marks the new release `--latest=false` so the newer release stays current. Branch builds (nightly prereleases on every non-`master` branch push) are unaffected.

The workflow refuses to build a branch named `stable` or one matching `vX.Y.Z`; those names are reserved for stable-channel releases.
