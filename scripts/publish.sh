#!/usr/bin/env bash
# Publishes every workspace crate to crates.io in dependency order.
#
# Versions already on crates.io are skipped, so a release that failed
# halfway (network, rate limit) can simply be re-run. `cargo publish` waits
# for each crate to appear in the index before returning, so the next crate
# can depend on it.
#
# Usage: scripts/publish.sh            # needs CARGO_REGISTRY_TOKEN
#        scripts/publish.sh --dry-run  # package and verify only
set -euo pipefail

# Dependencies before dependents.
CRATES=(rok-plan rok-render rok-edit rok-journal rok-locate rok-check rok-gen)

if [[ "${1:-}" == "--dry-run" ]]; then
  exec cargo publish --workspace --locked --dry-run
fi

: "${CARGO_REGISTRY_TOKEN:?CARGO_REGISTRY_TOKEN must be set}"

version_of() {
  cargo metadata --no-deps --format-version 1 \
    | jq -r --arg name "$1" '.packages[] | select(.name == $name) | .version'
}

for crate in "${CRATES[@]}"; do
  version="$(version_of "$crate")"
  status="$(curl -s -o /dev/null -w '%{http_code}' \
    -A 'rok-gen release (github.com/ateeq1999/rok-gen)' \
    "https://crates.io/api/v1/crates/${crate}/${version}")"
  if [[ "$status" == "200" ]]; then
    echo "skip     ${crate} ${version} (already on crates.io)"
    continue
  fi
  echo "publish  ${crate} ${version}"
  cargo publish -p "$crate" --locked
done
