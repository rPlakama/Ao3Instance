#!/bin/sh
# FlareSolverr is a host runtime service, never a build dependency.
#
# Usage: scripts/check-no-solver-build-dep.sh [--closure]
#   --closure also asks nix whether the package closure references it.
#
# Cargo.toml/Cargo.lock are strict: any reference means a crate or vendored tool.
# flake.nix legitimately names the solver only as runtime wiring: the
# `flaresolverrPort` NixOS-module option, its `AO3I_FLARESOLVERR*` environment
# variables, and the flake check that asserts that wiring. Those lines are
# allowlisted below; anything else (buildInputs, nativeBuildInputs, devShell,
# package inputs) still fails.
set -eu

fail=0

# Strict: a Rust dependency must never mention the solver.
for file in Cargo.toml Cargo.lock; do
    if [ -f "$file" ] && grep -n -i 'flaresolverr' "$file" >&2; then
        echo "unexpected FlareSolverr reference in $file" >&2
        fail=1
    fi
done

# flake.lock is a strict lock file: a solver input would show up here.
if [ -f flake.lock ] && grep -n -i 'flaresolverr' flake.lock >&2; then
    echo "unexpected FlareSolverr reference in flake.lock" >&2
    fail=1
fi

# flake.nix: only the documented runtime-wiring names are allowed.
if [ -f flake.nix ]; then
    unexpected=$(grep -n -i 'flaresolverr' flake.nix \
        | grep -v -i -E 'flaresolverrPort|AO3I_FLARESOLVERR|ao3instance-module-flaresolverr-port' \
        || true)
    if [ -n "$unexpected" ]; then
        printf '%s\n' "$unexpected" >&2
        echo "unexpected FlareSolverr reference in flake.nix" >&2
        fail=1
    fi
fi

if [ "${1:-}" = "--closure" ]; then
    if nix path-info -r .#default 2>/dev/null | grep -q -i 'flaresolverr'; then
        echo "FlareSolverr appears in the nix build closure" >&2
        fail=1
    fi
fi

if [ "$fail" -eq 0 ]; then
    echo "ok: FlareSolverr is not a build dependency"
fi
exit "$fail"
