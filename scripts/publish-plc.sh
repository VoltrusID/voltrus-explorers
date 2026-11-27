#!/bin/bash
# Publish a Voltrus PLC desktop release built by the "Release Voltrus PLC" workflow (release-plc.yml) to its download
# worker, dl-voltrus-plc.voltrus.id: files into the R2 bucket voltrus-plc-releases, rows into D1 voltrus-plc-releases-db.
#
#   scripts/publish-plc.sh <version> --run <run-id> [--notes "text"] [--dry-run]
#
# The universal DMG is recorded for darwin-aarch64 and darwin-x86_64, the zip for windows-x86_64, the .deb for
# linux-x86_64; /latest?platform=mac-arm|mac-intel|windows|linux serve them. No update packages are recorded, so the
# in-app updater of older versions sees no update (2.0.x were signed with a key that is no longer available; from
# 2.1.0 the app trusts the updater key used by CI, TAURI_SIGNING_PRIVATE_KEY).
# Runs with your own Cloudflare login (`npx wrangler login`, or CLOUDFLARE_API_TOKEN) and `gh auth login`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="${REPO:-VoltrusID/voltrus-explorers}"
export CLOUDFLARE_ACCOUNT_ID="${CLOUDFLARE_ACCOUNT_ID:-b7e6f8457bd39fa37a7eea5249799754}"
BUCKET="voltrus-plc-releases"; DB="voltrus-plc-releases-db"; BASE="https://dl-voltrus-plc.voltrus.id"

VERSION="${1:-}"; shift || true
RUN=""; NOTES="Bug fixes and improvements."; DRY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --run)     RUN="${2:?--run needs a run id}"; shift 2 ;;
    --notes)   NOTES="${2:?--notes needs text}"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    *) echo "unknown option: $1" >&2; exit 1 ;;
  esac
done
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && -n "$RUN" ]] || { sed -n '5p' "$0" | sed 's/^# //' >&2; exit 1; }

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
gh run download "$RUN" -R "$REPO" -D "$WORK"
DMG="$(ls "$WORK"/voltrus-plc-macos/*-"$VERSION"-universal.dmg)"
ZIP="$(ls "$WORK"/voltrus-plc-windows/Voltrus_PLC_"$VERSION"_x64.zip)"
DEB="$(ls "$WORK"/voltrus-plc-linux/*_"$VERSION"_amd64.deb)"

# Same file names as the earlier releases (Voltrus_PLC_<version>_<arch>.<ext>).
declare -A KEYS=(
  ["$DMG"]="Voltrus_PLC_${VERSION}_universal.dmg"
  ["$ZIP"]="Voltrus_PLC_${VERSION}_x64.zip"
  ["$DEB"]="Voltrus_PLC_${VERSION}_amd64.deb"
)
size() { stat -c%s "$1" 2>/dev/null || stat -f%z "$1"; }
sql() { if (( DRY )); then echo "SQL: $1"; else npx wrangler d1 execute "$DB" --remote --command "$1"; fi; }
esc() { printf "%s" "$1" | sed "s/'/''/g"; }

for f in "$DMG" "$ZIP" "$DEB"; do
  echo "==> ${KEYS[$f]} ($(size "$f") bytes)"
  (( DRY )) || npx wrangler r2 object put "$BUCKET/${KEYS[$f]}" --file "$f" --remote
done

# Publishing the same version again replaces its rows.
existing="$(npx wrangler d1 execute "$DB" --remote --json --command "SELECT version FROM releases WHERE short_version_string = '$VERSION'" \
  | python3 -c 'import json,sys; r=json.load(sys.stdin)[0]["results"]; print(r[0]["version"] if r else "")')"
if [[ -n "$existing" ]]; then
  NUM="$existing"
  sql "DELETE FROM release_files WHERE version = $NUM; DELETE FROM releases WHERE version = $NUM;"
else
  NUM="$(npx wrangler d1 execute "$DB" --remote --json --command "SELECT COALESCE(MAX(version), 0) + 1 AS n FROM releases" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["results"][0]["n"])')"
fi
DMG_KEY="${KEYS[$DMG]}"; ZIP_KEY="${KEYS[$ZIP]}"; DEB_KEY="${KEYS[$DEB]}"
NOW="$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"
sql "INSERT INTO releases (version, short_version_string, title, description, enclosure_url, ed_signature, length, pub_date)
     VALUES ($NUM, '$VERSION', 'Voltrus PLC $VERSION', '$(esc "$NOTES")', '$BASE/releases/$DMG_KEY', 'unsigned', $(size "$DMG"), '$NOW');
     INSERT INTO release_files (version, platform, filename, r2_key, size, signature) VALUES
       ($NUM, 'darwin-aarch64', '$DMG_KEY', '$DMG_KEY', $(size "$DMG"), NULL),
       ($NUM, 'darwin-x86_64', '$DMG_KEY', '$DMG_KEY', $(size "$DMG"), NULL),
       ($NUM, 'windows-x86_64', '$ZIP_KEY', '$ZIP_KEY', $(size "$ZIP"), NULL),
       ($NUM, 'linux-x86_64', '$DEB_KEY', '$DEB_KEY', $(size "$DEB"), NULL);"
echo "==> Voltrus PLC $VERSION published as release $NUM: $BASE/latest?platform=mac-arm|mac-intel|windows|linux"
