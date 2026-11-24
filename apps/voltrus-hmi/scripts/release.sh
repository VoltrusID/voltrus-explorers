#!/usr/bin/env bash
# DEPRECATED — do not use.
#
# This legacy pipeline had real defects: fake updater signing (wrote the
# literal string "tauri-signed" instead of a minisign signature), an appcast
# enclosure_url missing the /releases/ prefix (404), and a Tauri key path
# that did not match release-local.sh's (~/.config/tauri/).
#
# The maintained pipeline is ./release-local.sh:
#   ./release-local.sh                 # all platforms
#   ./release-local.sh --target aarch64
#   NOTARIZE=1 ./release-local.sh      # production
set -euo pipefail

echo "error: scripts/release.sh is deprecated and intentionally does nothing." >&2
echo "  Use scripts/release-local.sh instead." >&2
exit 1
