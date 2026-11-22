#!/bin/sh
# FlareSolverr is a host runtime service, never a build dependency.
#
# Usage: scripts/check-no-solver-build-dep.sh [--closure]
#   --closure also asks nix whether the package closure references it.
set -eu

fail=0
for file in Cargo.toml Cargo.lock flake.nix flake.lock; do
    if [ -f "$file" ] && grep -n -i 'flaresolverr' "$file" >&2; then
        echo "unexpected FlareSolverr reference in $file" >&2
        fail=1
    fi
done

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
