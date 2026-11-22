---
name: release
description: Cut a release. Use when a nix build passes and the binary has been tested, or when asked to release, tag, or bump the version.
---

# Release logic

Releases are gated by `nix build`. No green build, no release.

## Steps

1. Run `nix build` (and `nix flake check` if the project has checks). It must succeed.
2. Run the built binary and confirm it works properly.
3. Bump the version by **0.0.1**, counting like decimal digits (each part rolls over after 9):
   `0.7.9` → `0.8.0` → `0.8.1`, and `0.9.9` → `1.0.0`.
4. Update the version in the project (`Cargo.toml`, `flake.nix`, etc.), then rebuild so lockfiles stay consistent.
5. Commit, tag, and push:

       git commit -am "release: vX.Y.Z"
       git tag -a vX.Y.Z -m "<title>"
       git push && git push --tags

## Title

If none is given, use one technical phrase summarizing the changes since the previous release (find them with `git log <prev-tag>..HEAD --oneline`).

Example: `Add window screenshot support and inherit shared pipeline helpers`

Never tag a version that failed to build or was not tested.
