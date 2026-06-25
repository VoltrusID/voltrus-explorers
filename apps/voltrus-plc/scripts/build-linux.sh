#!/bin/bash
set -euo pipefail

# ============================================================
# Voltrus PLC — Build Linux via Docker (OrbStack)
# ============================================================
# Cross-compiles Linux AppImage/deb inside a Docker container.
#
# Usage:
#   ./scripts/build-linux.sh                       # Build only
#   ./scripts/build-linux.sh --upload              # Build + upload to R2
#   SKIP_BUILD=1 ./scripts/build-linux.sh --upload # Upload existing

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
WORKER_DIR="$PROJECT_DIR/cloudflare-worker"
APP_NAME="Voltrus PLC"
R2_BUCKET="voltrus-plc-releases"
R2_PUBLIC_BASE_URL="${R2_PUBLIC_BASE_URL:-https://dl-voltrus-plc.voltrus.id}"
SKIP_BUILD="${SKIP_BUILD:-0}"
BUILD_NUMBER="${BUILD_NUMBER:-3}"
UPLOAD="0"
VERSION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --upload)        UPLOAD=1; shift ;;
    --build-number)  BUILD_NUMBER="${2:-3}"; shift 2 ;;
    *)               VERSION="$1"; shift ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  VERSION=$(grep '"version"' "$PROJECT_DIR/src-tauri/tauri.conf.json" | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/')
fi

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { echo -e "${BLUE}[*]${NC} $1" >&2; }
ok()    { echo -e "${GREEN}[✓]${NC} $1" >&2; }
warn()  { echo -e "${YELLOW}[!]${NC} $1" >&2; }
err()   { echo -e "${RED}[✗]${NC} $1" >&2; }
step()  { echo -e "\n${BLUE}━━━ $1 ━━━${NC}" >&2; }

LINUX_OUT_DIR="$MONOREPO_DIR/target/linux-release"
CARGO_CACHE="$MONOREPO_DIR/target/linux-cargo-cache"
mkdir -p "$LINUX_OUT_DIR" "$CARGO_CACHE"

# ============================================================
# BUILD
# ============================================================

if [[ "$SKIP_BUILD" != "1" ]]; then
  step "Building Linux x86_64 via Docker"

  if ! docker info &>/dev/null; then
    err "Docker not running. Start OrbStack first."
    exit 1
  fi

  DOCKER_IMAGE="voltrus-plc-linux-builder"
  DOCKERFILE="$SCRIPT_DIR/Dockerfile.linux"

  info "Building Docker image (cached after first run)..."
  docker build --platform linux/amd64 -t "$DOCKER_IMAGE" -f "$DOCKERFILE" "$SCRIPT_DIR" >&2

  info "Building Tauri app inside container..."
  docker run --rm --platform linux/amd64 \
    -v "$MONOREPO_DIR:/workspace" \
    -v "$CARGO_CACHE:/cargo-cache" \
    -w /workspace \
    -e HOME=/root \
    -e CI=true \
    -e CARGO_HOME=/cargo-cache \
    "$DOCKER_IMAGE" \
    bash -c "
      export PATH=\"/cargo-cache/bin:\$PATH\"
      pnpm install 2>&1 | tail -3
      cd apps/voltrus-plc
      pnpm tauri build 2>&1 | tail -25
    " >&2

  info "Collecting Linux artifacts..."

  FOUND=0
  for pattern in "appimage/*.AppImage" "deb/*.deb"; do
    for f in "$MONOREPO_DIR/target/release/bundle"/$pattern; do
      if [[ -f "$f" ]]; then
        cp "$f" "$LINUX_OUT_DIR/"
        ok "Collected: $(basename "$f")"
        FOUND=1
      fi
    done
  done

  if [[ "$FOUND" == "0" ]]; then
    err "No Linux artifacts found in $MONOREPO_DIR/target/release/bundle/"
    err "Checked: appimage/*.AppImage, deb/*.deb"
    exit 1
  fi
else
  warn "Skipping build (SKIP_BUILD=1)"
fi

# ============================================================
# UPLOAD
# ============================================================

if [[ "$UPLOAD" != "1" ]]; then
  step "Build Complete (upload skipped)"
  ls -la "$LINUX_OUT_DIR/"
  echo ""
  echo -e "  ${YELLOW}To upload:${NC}  SKIP_BUILD=1 ./scripts/build-linux.sh --upload"
  echo ""
  exit 0
fi

step "Uploading to R2 & D1"

if [[ ! -d "$WORKER_DIR/node_modules" ]]; then
  cd "$WORKER_DIR" && npm install && cd "$PROJECT_DIR"
fi

cd "$WORKER_DIR"

ARTIFACT_PATH=""
for f in "$LINUX_OUT_DIR/"*.deb "$LINUX_OUT_DIR/"*.AppImage; do
  if [[ -f "$f" ]]; then
    ARTIFACT_PATH="$f"
    break
  fi
done

if [[ -z "$ARTIFACT_PATH" ]]; then
  err "No Linux artifact found in $LINUX_OUT_DIR/"
  exit 1
fi

FILENAME="$(basename "$ARTIFACT_PATH")"
NORMALIZED="${FILENAME// /_}"
if [[ "$NORMALIZED" != "$FILENAME" ]]; then
  cp "$ARTIFACT_PATH" "$LINUX_OUT_DIR/$NORMALIZED"
  ARTIFACT_PATH="$LINUX_OUT_DIR/$NORMALIZED"
  FILENAME="$NORMALIZED"
fi

FILESIZE=$(stat -f%z "$ARTIFACT_PATH" 2>/dev/null || stat -c%s "$ARTIFACT_PATH")
PLATFORM="linux-x86_64"

info "Uploading $FILENAME to R2..."
npx wrangler r2 object put "$R2_BUCKET/$FILENAME" --file="$ARTIFACT_PATH" --remote
ok "Uploaded: $FILENAME"

info "Inserting $PLATFORM into D1 release_files..."
npx wrangler d1 execute voltrus-plc-releases-db --remote --command \
  "INSERT OR REPLACE INTO release_files (version, platform, filename, r2_key, size) VALUES ($BUILD_NUMBER, '$PLATFORM', '$FILENAME', '$FILENAME', $FILESIZE);"
ok "D1 record: $PLATFORM"

ENCLOSURE_URL="${R2_PUBLIC_BASE_URL%/}/releases/$FILENAME"
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")
TITLE="$APP_NAME $VERSION"

info "Updating legacy releases table..."
npx wrangler d1 execute voltrus-plc-releases-db --remote --command \
  "INSERT OR REPLACE INTO releases (version, short_version_string, title, description, enclosure_url, ed_signature, length, pub_date) VALUES ($BUILD_NUMBER, '$VERSION', '$TITLE', 'Bug fixes and improvements.', '$ENCLOSURE_URL', 'unsigned', $FILESIZE, '$PUB_DATE');"
ok "Appcast record updated"

cd "$PROJECT_DIR"

FILE_SIZE_MB=$(echo "scale=1; $FILESIZE / 1048576" | bc)

echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║  Linux Build Complete — v$VERSION              ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  Artifact:  ${BLUE}$FILENAME${NC} (${FILE_SIZE_MB} MB)"
echo -e "  Platform:  ${BLUE}$PLATFORM${NC}"
echo -e "  Download:  ${BLUE}${R2_PUBLIC_BASE_URL}/latest?platform=linux${NC}"
echo ""
