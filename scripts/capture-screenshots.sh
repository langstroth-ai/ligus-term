#!/usr/bin/env bash
# Screenshots of lt for the website, the docs and the READMEs, from the demo
# backend (no device, no network): every gallery view in Pollen light and
# dark, at 110x32.
#
#   scripts/capture-screenshots.sh [OUT_DIR]     (default: screenshots-out)
#
# Run it from a checkout of the release being documented (for example
# `git worktree add ../lt-v0.3.0 v0.3.0`). If that tag's
# examples/screenshots.rs predates LT_SHOT_MODE, copy this branch's
# example over it first: the example is dev-only and not part of `lt`.
#
# Writes, for every view:
#   OUT/svg/lt-<view>-<mode>.svg
#   OUT/png/lt-<view>-<mode>.png       2x, pngquant'd   (docs)
#   OUT/png/lt-<view>-<mode>@1x.png    1x, pngquant'd   (READMEs)
#   OUT/webp/lt-<view>-<mode>.webp     1x               (website)
#   OUT/webp/lt-<view>-<mode>@2x.webp  2x               (website)
#
# Needs cargo, rsvg-convert, pngquant and cwebp
# (brew install librsvg pngquant webp).
set -euo pipefail

out=${1:-screenshots-out}
for tool in cargo rsvg-convert pngquant cwebp; do
  command -v "$tool" >/dev/null || { echo "$tool not found (brew install librsvg pngquant webp)" >&2; exit 3; }
done
mkdir -p "$out/svg" "$out/png" "$out/webp"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for mode in light dark; do
  LT_SHOT_MODE=$mode cargo run --quiet --example screenshots -- --gallery "$tmp/$mode" >/dev/null
  for svg in "$tmp/$mode"/*-110x32.svg; do
    view=$(basename "$svg" -110x32.svg)
    name="lt-$view-$mode"
    cp "$svg" "$out/svg/$name.svg"
    rsvg-convert -z 2 "$svg" -o "$out/png/$name.png"
    rsvg-convert -z 1 "$svg" -o "$out/png/$name@1x.png"
    cwebp -quiet -q 82 -m 6 "$out/png/$name.png" -o "$out/webp/$name@2x.webp"
    cwebp -quiet -q 82 -m 6 "$out/png/$name@1x.png" -o "$out/webp/$name.webp"
  done
done
pngquant --quality=65-90 --speed 1 --force --ext .png "$out"/png/*.png
echo "$(ls "$out/png" | wc -l | tr -d ' ') PNGs in $out/png, WebP in $out/webp"
