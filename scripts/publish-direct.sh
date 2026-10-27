#!/bin/bash
# Publish the notarized Developer ID DMG of a direct-download app to its download worker: R2 bucket + D1 releases.
#
#   scripts/publish-direct.sh <app> <version> [--run <run-id> | <path-to.dmg>] [--notes "text"] [--dry-run]
#
# <app> is serial-monitor or ble-inspector: they are not on GitHub Releases or the App Store; the console
# (console.voltrus.id) and voltrus.id link https://dl-<app>.voltrus.id/latest, which this script feeds.
# The DMG is the `dmg` output of the Release workflow (scripts/build-macos.sh <app> dmg --notarize, built with
# `--features license`). It comes from, in order:
#   --run <id>      the workflow run's artifact (gh run download; needs `gh auth login`)
#   <path>          a local .dmg
#   neither         build/<app>/dmg/*-<version>-universal.dmg from a local build
#
# Runs locally with your own Cloudflare login (`npx wrangler login`, or CLOUDFLARE_API_TOKEN). Same R2 + D1 layout as
# the legacy MacTools release.sh: the DMG lands in the app's R2 bucket under its file name, `releases` gets the
# version row (appcast), and `release_files` gets darwin-aarch64 and darwin-x86_64 rows (the DMG is universal),
# which is what /latest and /downloads of the worker serve. Publishing a version again replaces its rows.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO="${REPO:-VoltrusID/voltrus-explorers}"
ACCOUNT_ID="b7e6f8457bd39fa37a7eea5249799754"
usage() { sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 1; }

APP="${1:-}"; VERSION="${2:-}"
[[ -n "$APP" && -n "$VERSION" ]] || usage
shift 2
RUN=""; SRC=""; NOTES="Bug fixes and improvements."; DRY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --run)     RUN="${2:?--run needs a run id}"; shift 2 ;;
    --notes)   NOTES="${2:?--notes needs text}"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    -*)        echo "unknown option: $1" >&2; usage ;;
    *)         [[ -z "$SRC" ]] || usage; SRC="$1"; shift ;;
  esac
done
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "version must look like 1.6.0: $VERSION" >&2; exit 1; }
[[ -z "$RUN" || -z "$SRC" ]] || { echo "give --run or a path, not both" >&2; exit 1; }

case "$APP" in
  serial-monitor)
    PRODUCT="Serial Monitor"; BASE="https://dl-serial-monitor.voltrus.id"
    BUCKET="serial-monitor-releases"; DB="serial-monitor-releases-db"; DB_ID="5aa698d5-26a1-4aba-8a19-818b056bfa85"
    NAME_RE='serial' ;;
  ble-inspector)
    PRODUCT="BLE Inspector"; BASE="https://dl-ble-inspector.voltrus.id"
    BUCKET="ble-inspector-releases"; DB="ble-inspector-releases-db"; DB_ID="ab0187c9-a3bf-4e64-b484-f99268c644f4"
    NAME_RE='ble' ;;
  *) echo "not a direct-download app: $APP (serial-monitor | ble-inspector)" >&2; exit 1 ;;
esac

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
step() { echo "==> $*"; }

# ---- find the DMG ---------------------------------------------------------------------------------------------
if [[ -n "$RUN" ]]; then
  step "downloading the artifact of run $RUN"
  gh run download "$RUN" -R "$REPO" --pattern "$APP-macos-*" -D "$TMP/artifact"
  DMGS=( $(find "$TMP/artifact" -name '*.dmg') )
  [[ ${#DMGS[@]} -eq 1 ]] || { echo "expected one .dmg in run $RUN, found ${#DMGS[@]} (was the run started with dmg=true for $APP?)" >&2; exit 1; }
  SRC="${DMGS[0]}"
elif [[ -z "$SRC" ]]; then
  DMGS=( "$ROOT/build/$APP/dmg/"*"-$VERSION-universal.dmg" )
  [[ -f "${DMGS[0]}" ]] || { echo "no local build/$APP/dmg/*-$VERSION-universal.dmg; pass --run <id> or a path" >&2; exit 1; }
  SRC="${DMGS[0]}"
fi
[[ -f "$SRC" && "$SRC" == *.dmg ]] || { echo "not a .dmg file: $SRC" >&2; exit 1; }

KEY="$(basename "$SRC")"; KEY="${KEY// /_}"
[[ "$KEY" == *"-$VERSION-"* || "$KEY" == *"_${VERSION}_"* ]] || { echo "file name $KEY does not carry version $VERSION" >&2; exit 1; }
[[ "$KEY" =~ ^[A-Za-z0-9._-]+$ ]] || { echo "unexpected characters in file name $KEY" >&2; exit 1; }
# The worker ignores files whose name does not match this (guards against another product's file in its bucket).
echo "$KEY" | grep -qiE "$NAME_RE" || { echo "file name $KEY must contain '$NAME_RE' or the worker will not serve it" >&2; exit 1; }
SIZE="$(stat -f%z "$SRC" 2>/dev/null || stat -c%s "$SRC")"

if [[ "$(uname)" == Darwin ]]; then
  step "checking notarization"
  xcrun stapler validate "$SRC"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$SRC"
else
  echo "!! not on macOS: skipping the stapler/Gatekeeper check of the DMG" >&2
fi

# ---- wrangler -------------------------------------------------------------------------------------------------
CONF="$TMP/wrangler.toml"
cat > "$CONF" <<EOF
name = "publish-direct"
account_id = "$ACCOUNT_ID"
compatibility_date = "2024-01-01"

[[d1_databases]]
binding = "DB"
database_name = "$DB"
database_id = "$DB_ID"

[[r2_buckets]]
binding = "BUCKET"
bucket_name = "$BUCKET"
EOF
export CLOUDFLARE_ACCOUNT_ID="$ACCOUNT_ID"
read -r -a WRANGLER <<< "${WRANGLER:-npx --yes wrangler}"
wr() { "${WRANGLER[@]}" "$@" -c "$CONF"; }
d1_value() { # first column of the first row of a SELECT, empty when there is none
  wr d1 execute "$DB" --remote --json --command "$1" \
    | python3 -c 'import json,sys; r=json.load(sys.stdin)[0]["results"]; print(next(iter(r[0].values()), "") if r else "")'
}

# ---- build number: reuse the row of this version, else the next free one ---------------------------------------------
if (( DRY )); then
  BUILD="<next build number>"
else
  BUILD="$(d1_value "SELECT version FROM releases WHERE short_version_string = '$VERSION' LIMIT 1")"
  if [[ -z "$BUILD" ]]; then
    MAX="$(d1_value "SELECT MAX(v) FROM (SELECT version AS v FROM releases UNION ALL SELECT version AS v FROM release_files)")"
    BUILD="$(( ${MAX:-0} + 1 ))"
  fi
fi

esc() { printf '%s' "${1//\'/\'\'}"; }
URL="$BASE/releases/$KEY"
PUB_DATE="$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"
SQL="$TMP/publish.sql"
cat > "$SQL" <<EOF
INSERT INTO releases (version, short_version_string, title, description, enclosure_url, ed_signature, length, pub_date)
VALUES ($BUILD, '$VERSION', '$(esc "$PRODUCT $VERSION")', '$(esc "$NOTES")', '$(esc "$URL")', 'unsigned', $SIZE, '$PUB_DATE')
ON CONFLICT(version) DO UPDATE SET short_version_string = excluded.short_version_string, title = excluded.title,
  description = excluded.description, enclosure_url = excluded.enclosure_url, ed_signature = excluded.ed_signature,
  length = excluded.length, pub_date = excluded.pub_date;
DELETE FROM release_files WHERE version = $BUILD AND platform IN ('darwin-aarch64', 'darwin-x86_64');
INSERT INTO release_files (version, platform, filename, r2_key, size) VALUES ($BUILD, 'darwin-aarch64', '$(esc "$KEY")', '$(esc "$KEY")', $SIZE);
INSERT INTO release_files (version, platform, filename, r2_key, size) VALUES ($BUILD, 'darwin-x86_64', '$(esc "$KEY")', '$(esc "$KEY")', $SIZE);
EOF

if (( DRY )); then
  echo "dry run: $PRODUCT $VERSION, $SRC ($SIZE bytes)"
  echo "+ wrangler r2 object put $BUCKET/$KEY --file $SRC --content-type application/x-apple-diskimage --remote"
  echo "+ wrangler d1 execute $DB --remote --file publish.sql:"
  cat "$SQL"
  exit 0
fi

step "uploading $KEY to R2 bucket $BUCKET"
wr r2 object put "$BUCKET/$KEY" --file "$SRC" --content-type application/x-apple-diskimage --remote

step "recording $PRODUCT $VERSION (build $BUILD) in D1 $DB"
wr d1 execute "$DB" --remote --file "$SQL"

# ---- check what the public sees ---------------------------------------------------------------------------------
step "checking $BASE/latest"
LOCATION="$(curl -s -o /dev/null -w '%{redirect_url}' "$BASE/latest")"
echo "    /latest -> ${LOCATION:-(no redirect)}"
if [[ "$LOCATION" != "$URL" ]]; then
  echo "!! /latest does not point at the new file yet; compare with a newer semver already in the bucket, or the worker may be cached" >&2
  exit 1
fi
echo "published: $BASE/latest -> $KEY"
