#!/usr/bin/env bash
# Copy the shared colour files from a ligus checkout into lt, or check the
# copies match (--check). The ligus repo generates lt's colour table from
# its brand tokens; lt keeps the table, the JSON it came from and the theme
# file schema, and tests/contrast.rs checks the table against the JSON.
#
#   scripts/sync-theme-tokens.sh [--check] [LIGUS_CHECKOUT]   (default ../ligus)
set -euo pipefail
check=0
if [[ "${1:-}" == "--check" ]]; then check=1; shift; fi
ligus="${1:-$(dirname "$0")/../../ligus}"
here="$(cd "$(dirname "$0")/.." && pwd)"
[[ -f "$ligus/assets/branding/ligus-brand-tokens.json" ]] || { echo "no ligus checkout at $ligus" >&2; exit 3; }
# The ligus generator's own check: its generated files match its JSON.
node "$ligus/assets/branding/tokens/generate.mjs" --check
pairs=(
  "assets/branding/generated/lt/ligus_tokens.rs:src/theme_tokens.rs"
  "assets/branding/ligus-brand-tokens.json:assets/branding/ligus-brand-tokens.json"
  "docs-site/docs/schemas/ligus-theme.v1.json:assets/themes/ligus-theme.v1.json"
)
stale=0
for p in "${pairs[@]}"; do
  from="$ligus/${p%%:*}" to="$here/${p##*:}"
  if [[ $check == 1 ]]; then
    cmp -s "$from" "$to" || { echo "stale: ${p##*:}" >&2; stale=1; }
  else
    mkdir -p "$(dirname "$to")" && cp "$from" "$to" && echo "copied ${p##*:}" >&2
  fi
done
if [[ $check == 1 && $stale == 1 ]]; then echo "run: scripts/sync-theme-tokens.sh $ligus" >&2; exit 1; fi
