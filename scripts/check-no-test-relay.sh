#!/usr/bin/env bash
# Fails if lt's normal build graph enables a test-only feature of the Ligus
# crates: ligus-runtime's `test-relay` (the in-process mock relay, which
# admits any request) or `test-presence` (a recovery key that answers every
# passkey prompt). The integration tests enable `test-relay` through a
# dev-dependency only; the resolver keeps dev features out of `cargo build`.
#
# Also checks that the test graph does enable `test-relay`, so this check
# can't pass vacuously (a renamed feature, a dropped dev-dependency).
set -euo pipefail
cd "$(dirname "$0")/.."
# The runtime crate's package name.
crate=ligus-runtime

fail=0
for flags in "" "--no-default-features" "--all-features"; do
  out=$(cargo tree --locked -p ligus-term $flags -e features,normal,build -i "$crate" 2>/dev/null || true)
  if [ -z "$out" ]; then
    echo "check-no-test-relay: cargo tree ($flags) printed nothing" >&2
    fail=1
    continue
  fi
  for feat in test-relay test-presence; do
    if grep -q "\"$feat\"" <<<"$out"; then
      echo "check-no-test-relay: lt ($flags) enables $crate/$feat in a normal build:" >&2
      grep "\"$feat\"" <<<"$out" >&2
      fail=1
    fi
  done
done

if ! cargo tree --locked -p ligus-term -e features,normal,dev -i "$crate" 2>/dev/null | grep -q '"test-relay"'; then
  echo "check-no-test-relay: the control failed (the test graph no longer shows test-relay; update this script)" >&2
  fail=1
fi

[ "$fail" -eq 0 ] && echo "check-no-test-relay: ok"
exit "$fail"
