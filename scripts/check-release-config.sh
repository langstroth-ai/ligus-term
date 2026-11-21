#!/usr/bin/env bash
# Refuse to release unless the tag is v<the version lt is built as>.
#
#   scripts/check-release-config.sh v0.1.1
#
# Checks the tag is a plain release tag (v1.2.3 or v1.2.3-rc.1), that
# Cargo.toml's [package] version is the tag's version, and that Cargo.lock
# records the same version for ligus-term (a bump without a lock update would
# fail `cargo build --locked` only after the draft exists). cut-release.yml
# runs it before tagging, and release.yml before building or uploading.
#
# LT_RELEASE_ROOT points it at another tree (used by the tests).
set -euo pipefail

tag="${1:?usage: check-release-config.sh <tag>}"
root="${LT_RELEASE_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"

if ! [[ "$tag" =~ ^v([0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?)$ ]]; then
  echo "::error::tag '$tag' is not a release tag like v0.1.1"
  exit 1
fi
want="${BASH_REMATCH[1]}"

# The first version = line after [package] (before any other table).
toml=$(awk '/^\[package\]/{p=1; next} /^\[/{p=0} p && /^version *=/{gsub(/.*= *"|".*/, ""); print; exit}' "$root/Cargo.toml")
# The version line right after name = "ligus-term" in the lock file.
lock=$(awk '/^name = "ligus-term"$/{getline; gsub(/.*= *"|".*/, ""); print; exit}' "$root/Cargo.lock")

fail=0
if [ "$toml" != "$want" ]; then
  echo "::error::Cargo.toml has version '$toml', not $want (the tag $tag)"
  fail=1
fi
if [ "$lock" != "$want" ]; then
  echo "::error::Cargo.lock has ligus-term '$lock', not $want: run cargo update -p ligus-term --offline"
  fail=1
fi
[ "$fail" -eq 0 ] && echo "release config OK: $tag = Cargo.toml = Cargo.lock"
exit "$fail"
