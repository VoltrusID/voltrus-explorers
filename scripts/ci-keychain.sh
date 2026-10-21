#!/bin/bash
# CI only: install the App Store Connect API key and import signing identities into a temporary keychain.
#
#   scripts/ci-keychain.sh setup NAME...   # NAME = APPLE_DIST | MAC_INSTALLER | DEVELOPER_ID
#   scripts/ci-keychain.sh cleanup
#
# Reads ASC_KEY_ID + ASC_KEY_BASE64 and, per NAME, <NAME>_P12_BASE64 + <NAME>_P12_PASSWORD from the environment.
# Appends ASC_KEY_PATH to $GITHUB_ENV. Same steps as joyorogo's release-ios.yml.
set -euo pipefail

KC="$RUNNER_TEMP/release.keychain-db"
KEY_DIR="$HOME/.appstoreconnect/private_keys"

case "${1:-}" in
setup)
  shift
  umask 077
  mkdir -p "$KEY_DIR"
  printf '%s' "$ASC_KEY_BASE64" | base64 --decode > "$KEY_DIR/AuthKey_$ASC_KEY_ID.p8"
  echo "ASC_KEY_PATH=$KEY_DIR/AuthKey_$ASC_KEY_ID.p8" >> "$GITHUB_ENV"
  [[ $# -gt 0 ]] || exit 0 # key only (iOS: Tauri imports its own certificate)

  kcpass="$(uuidgen)"
  security create-keychain -p "$kcpass" "$KC"
  security set-keychain-settings -lut 21600 "$KC"
  security unlock-keychain -p "$kcpass" "$KC"
  for name in "$@"; do
    b64="${name}_P12_BASE64"; pass="${name}_P12_PASSWORD"
    printf '%s' "${!b64:?$b64 not set}" | base64 --decode > "$RUNNER_TEMP/$name.p12"
    security import "$RUNNER_TEMP/$name.p12" -k "$KC" -P "${!pass:?$pass not set}" \
      -T /usr/bin/codesign -T /usr/bin/productbuild -T /usr/bin/security
    rm -f "$RUNNER_TEMP/$name.p12"
  done
  security set-key-partition-list -S apple-tool:,apple: -s -k "$kcpass" "$KC" > /dev/null
  security list-keychains -d user -s "$KC" $(security list-keychains -d user | tr -d '"')
  security find-identity -v "$KC" | sed 's/^ *[0-9]*) [0-9A-F]* //'
  ;;
cleanup)
  rm -f "$KEY_DIR"/AuthKey_*.p8
  security delete-keychain "$KC" 2>/dev/null || true
  ;;
*)
  echo "usage: $0 setup NAME... | cleanup" >&2; exit 1 ;;
esac
