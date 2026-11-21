#!/usr/bin/env bash
# Developer ID signing and notarization of the macOS `lt` binaries, for
# release.yml's build job.
#
#   scripts/macos-sign.sh configure          # before the build
#   scripts/macos-sign.sh sign <binary>      # after it
#   scripts/macos-sign.sh notarize <binary>
#   scripts/macos-sign.sh cleanup            # always, at the end
#
# configure is a no-op (the build stays unsigned) when APPLE_CERTIFICATE is
# empty, the way the ligus release treats its signing secrets. When it is
# set, every other secret must be too (a half-configured release fails
# instead of shipping something in between): APPLE_CERTIFICATE (base64 .p12),
# APPLE_CERTIFICATE_PASSWORD, APPLE_SIGNING_IDENTITY ("Developer ID
# Application: ... (TEAMID)"), APPLE_TEAM_ID, and for notarization
# APPLE_API_KEY, APPLE_API_ISSUER and APPLE_API_KEY_P8 (the .p8 text).
# It imports the certificate into a throwaway keychain and writes to
# $GITHUB_ENV:
#   LIGUS_APPLE_TEAM_ID  read by ligus-runtime at build time (option_env!):
#                        a binary built with it keeps the keychain's unlock
#                        password only while it satisfies
#                          anchor apple generic and identifier "ai.ligus.term"
#                          and certificate leaf[subject.OU] = "<team>"
#                        so it must also be signed as below, never built with
#                        the team id and shipped unsigned.
#   LT_MACOS_SIGNED=1, LT_SIGN_KEYCHAIN
#
# sign uses the identifier lt names at startup (SIGNING_IDENTIFIER in
# src/backend/real/mod.rs, set_binary_identifier), the hardened runtime and
# a secure timestamp, then checks the signature satisfies that requirement.
#
# notarize submits a zip of the binary and waits for "Accepted". A bare
# Mach-O can't carry a stapled ticket; Gatekeeper looks the ticket up online.
#
# Secrets are read from the environment and never printed; the API key is a
# 0600 temp file removed on exit.
set -euo pipefail

IDENTIFIER="ai.ligus.term"

need() {
  local missing=()
  for v in "$@"; do
    [ -n "${!v:-}" ] || missing+=("$v")
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "::error::APPLE_CERTIFICATE is set but these secrets are empty: ${missing[*]}"
    exit 1
  fi
}

requirement() {
  printf 'anchor apple generic and identifier "%s" and certificate leaf[subject.OU] = "%s"' "$IDENTIFIER" "$1"
}

cmd="${1:?usage: macos-sign.sh configure|sign <bin>|notarize <bin>|cleanup}"
case "$cmd" in
  configure)
    if [ -z "${APPLE_CERTIFICATE:-}" ]; then
      echo "No APPLE_CERTIFICATE secret: building lt unsigned."
      exit 0
    fi
    need APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY APPLE_TEAM_ID APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_P8
    [[ "$APPLE_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] || { echo "::error::APPLE_TEAM_ID is not a 10-character team id"; exit 1; }
    kc="$RUNNER_TEMP/lt-signing.keychain-db"
    kc_pass=$(openssl rand -hex 24)
    p12="$(mktemp "$RUNNER_TEMP/cert.XXXXXX")"
    trap 'rm -f "$p12"' EXIT
    printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$p12"
    security create-keychain -p "$kc_pass" "$kc"
    security set-keychain-settings -lut 21600 "$kc"
    security unlock-keychain -p "$kc_pass" "$kc"
    security import "$p12" -f pkcs12 -k "$kc" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
    security set-key-partition-list -S apple-tool:,apple: -s -k "$kc_pass" "$kc" > /dev/null
    # Search it first, after the user's existing keychains.
    # shellcheck disable=SC2046
    security list-keychains -d user -s "$kc" $(security list-keychains -d user | tr -d '"')
    security find-identity -v -p codesigning "$kc" | grep -qF "$APPLE_SIGNING_IDENTITY" || {
      echo "::error::APPLE_SIGNING_IDENTITY is not a codesigning identity in APPLE_CERTIFICATE"
      exit 1
    }
    {
      echo "LIGUS_APPLE_TEAM_ID=$APPLE_TEAM_ID"
      echo "LT_MACOS_SIGNED=1"
      echo "LT_SIGN_KEYCHAIN=$kc"
    } >> "$GITHUB_ENV"
    echo "macOS signing and notarization configured (identifier $IDENTIFIER)."
    ;;
  sign)
    bin="${2:?usage: macos-sign.sh sign <binary>}"
    need APPLE_SIGNING_IDENTITY APPLE_TEAM_ID LT_SIGN_KEYCHAIN
    codesign --force --options runtime --timestamp --identifier "$IDENTIFIER" \
      --keychain "$LT_SIGN_KEYCHAIN" --sign "$APPLE_SIGNING_IDENTITY" "$bin"
    codesign --verify --strict --verbose=2 "$bin"
    codesign --display --verbose=2 "$bin" 2>&1 | grep -E '^(Identifier|TeamIdentifier|Authority|Timestamp|Runtime)'
    # The requirement ligus-runtime checks before trusting itself with the
    # keychain's unlock password.
    codesign --verify --strict -R="$(requirement "$APPLE_TEAM_ID")" "$bin"
    echo "signed $bin as $IDENTIFIER"
    ;;
  notarize)
    bin="${2:?usage: macos-sign.sh notarize <binary>}"
    need APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_P8
    key="$(mktemp "$RUNNER_TEMP/notary-key.XXXXXX")"
    zip="$RUNNER_TEMP/$(basename "$bin")-notarize.zip"
    trap 'rm -f "$key" "$zip"' EXIT
    chmod 600 "$key"
    printf '%s' "$APPLE_API_KEY_P8" > "$key"
    ditto -c -k --keepParent "$bin" "$zip"
    out="$(xcrun notarytool submit "$zip" --key "$key" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" \
      --wait --timeout 45m --output-format json)" || true
    echo "$out"
    status="$(python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("status",""))' <<<"$out" 2>/dev/null || true)"
    if [ "$status" != Accepted ]; then
      id="$(python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("id",""))' <<<"$out" 2>/dev/null || true)"
      if [ -n "$id" ]; then
        xcrun notarytool log "$id" --key "$key" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER" || true
      fi
      echo "::error::notarizing $(basename "$bin"): status '${status:-none}'"
      exit 1
    fi
    echo "notarized $bin"
    ;;
  cleanup)
    if [ -n "${LT_SIGN_KEYCHAIN:-}" ]; then
      security delete-keychain "$LT_SIGN_KEYCHAIN" 2>/dev/null || true
    fi
    ;;
  *)
    echo "usage: macos-sign.sh configure|sign <bin>|notarize <bin>|cleanup" >&2
    exit 2
    ;;
esac
