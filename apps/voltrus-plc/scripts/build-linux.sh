#!/bin/bash
set -euo pipefail

# ============================================================
# Voltrus PLC — Build Linux via Docker
# ============================================================
# Builds the Linux AppImage + .deb inside an Ubuntu 22.04 container
# (glibc 2.35) so the binaries run on older distros than the build host.
# Works with OrbStack on macOS or native Docker on Linux.
#
# Usage:
#   ./scripts/build-linux.sh                       # Build only
#   ./scripts/build-linux.sh --upload              # Build + upload to R2
#   SKIP_BUILD=1 ./scripts/build-linux.sh --upload # Upload existing
#   BUILD_NUMBER=1002000 ./scripts/build-linux.sh --upload

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
# Download worker lives in the private VoltrusID/voltrus-dl-workers repo (checked out next to this one).
WORKER_DIR="${VOLTRUS_DL_WORKERS:-$MONOREPO_DIR/../voltrus-dl-workers}/apps/voltrus-plc/cloudflare-worker"
APP_DIR_NAME="voltrus-plc"
APP_NAME="Voltrus PLC"
R2_BUCKET="voltrus-plc-releases"
D1_DATABASE="voltrus-plc-releases-db"
R2_PUBLIC_BASE_URL="${R2_PUBLIC_BASE_URL:-https://dl-voltrus-plc.voltrus.id}"
SKIP_BUILD="${SKIP_BUILD:-0}"
UPLOAD="0"
VERSION=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --upload)        UPLOAD=1; shift ;;
    --build-number)  BUILD_NUMBER="${2:-}"; shift 2 ;;
    *)               VERSION="$1"; shift ;;
  esac
done

if [[ -z "$VERSION" ]]; then
  VERSION=$(grep '"version"' "$PROJECT_DIR/src-tauri/tauri.conf.json" | head -1 | sed 's/.*"version": *"\([^"]*\)".*/\1/')
fi

# Default build number from semver: major*1_000_000 + minor*1_000 + patch (unique per version)
if [[ -z "${BUILD_NUMBER:-}" ]]; then
  IFS='.' read -r v_major v_minor v_patch <<< "${VERSION%%-*}"
  BUILD_NUMBER=$(( v_major * 1000000 + v_minor * 1000 + v_patch ))
fi

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
# target/ can be a (possibly dangling) symlink into a build cache
if [[ -L "$MONOREPO_DIR/target" ]]; then mkdir -p "$(readlink -f "$MONOREPO_DIR/target")"; fi
mkdir -p "$LINUX_OUT_DIR" "$CARGO_CACHE"

# Tauri names bundles "<productName>_<version>_amd64.{AppImage,deb}"; the shared
# monorepo bundle dir also holds other apps' artifacts, so match on our prefix.
ARTIFACT_PREFIX="${APP_NAME}_${VERSION}_"

# ============================================================
# BUILD
# ============================================================

if [[ "$SKIP_BUILD" != "1" ]]; then
  step "Building Linux x86_64 via Docker"

  if ! docker info &>/dev/null; then
    err "Docker not running (start OrbStack on macOS, or the docker service on Linux)."
    exit 1
  fi

  DOCKER_IMAGE="mactools-linux-builder"
  DOCKERFILE="$SCRIPT_DIR/Dockerfile.linux"

  info "Building Docker image (cached after first run)..."
  docker build --platform linux/amd64 -t "$DOCKER_IMAGE" -f "$DOCKERFILE" "$SCRIPT_DIR" >&2

  # On a Linux host the bind mounts are not uid-mapped: hand everything the
  # root container wrote back to the invoking user when done.
  HOST_UID="$(id -u)"
  HOST_GID="$(id -g)"

  # target/ may be a symlink to a cache dir outside the repo: mount the real
  # directory at the same absolute path so the symlink resolves in the container.
  EXTRA_MOUNTS=()
  if [[ -L "$MONOREPO_DIR/target" ]]; then
    TARGET_REAL="$(readlink -f "$MONOREPO_DIR/target")"
    mkdir -p "$TARGET_REAL"
    EXTRA_MOUNTS+=(-v "$TARGET_REAL:$TARGET_REAL")
  fi

  info "Building Tauri app inside container..."
  docker run --rm --platform linux/amd64 \
    -v "$MONOREPO_DIR:/workspace" \
    ${EXTRA_MOUNTS[@]+"${EXTRA_MOUNTS[@]}"} \
    -v "$CARGO_CACHE:/cargo-cache" \
    -w /workspace \
    -e HOME=/root \
    -e CI=true \
    -e npm_config_verify_deps_before_run=false \
    -e CARGO_HOME=/cargo-cache \
    -e APPIMAGE_EXTRACT_AND_RUN=1 \
    "$DOCKER_IMAGE" \
    bash -c "
      set -e
      export PATH=\"/cargo-cache/bin:\$PATH\"
      cleanup() {
        chown -R $HOST_UID:$HOST_GID /workspace/target/release /cargo-cache 2>/dev/null || true
        find /workspace/target/ -maxdepth 1 -user 0 -exec chown -h $HOST_UID:$HOST_GID {} + 2>/dev/null || true
        find /workspace -path /workspace/target -prune -o -user 0 -exec chown -h $HOST_UID:$HOST_GID {} + 2>/dev/null || true
      }
      trap cleanup EXIT
      # Reuse the host's node_modules when it is already a Linux x64 install
      # (a container pnpm install would fight over the store path).
      if [[ ! -d node_modules/.pnpm ]]; then pnpm install 2>&1 | tail -3; fi
      cd apps/$APP_DIR_NAME
      # Call the tauri CLI directly: 'pnpm tauri' verifies deps first and, seeing
      # a different store path, reinstalls the host's node_modules as root.
      ./node_modules/.bin/tauri build --bundles appimage,deb 2>&1 | tail -25
    " >&2

  info "Collecting Linux artifacts..."
  FOUND=0
  for f in "$MONOREPO_DIR/target/release/bundle/appimage/${ARTIFACT_PREFIX}"*.AppImage \
           "$MONOREPO_DIR/target/release/bundle/deb/${ARTIFACT_PREFIX}"*.deb; do
    if [[ -f "$f" ]]; then
      base="$(basename "$f")"
      cp "$f" "$LINUX_OUT_DIR/${base// /_}"
      ok "Collected: ${base// /_}"
      FOUND=1
    fi
  done

  if [[ "$FOUND" == "0" ]]; then
    err "No Linux artifacts for $APP_NAME $VERSION in $MONOREPO_DIR/target/release/bundle/"
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
  ls -la "$LINUX_OUT_DIR/" | grep -F "${ARTIFACT_PREFIX// /_}" || true
  echo ""
  echo -e "  ${YELLOW}To upload:${NC}  SKIP_BUILD=1 ./scripts/build-linux.sh --upload"
  echo ""
  exit 0
fi

step "Uploading to R2 & D1 (build number $BUILD_NUMBER)"

if [[ ! -d "$WORKER_DIR/node_modules" ]]; then
  cd "$WORKER_DIR" && npm install && cd "$PROJECT_DIR"
fi

cd "$WORKER_DIR"

# Prefer the .deb (much smaller; the AppImage bundles the whole WebKitGTK stack)
ARTIFACT_PATH=""
for f in "$LINUX_OUT_DIR/${ARTIFACT_PREFIX// /_}"*.deb "$LINUX_OUT_DIR/${ARTIFACT_PREFIX// /_}"*.AppImage; do
  if [[ -f "$f" ]]; then
    ARTIFACT_PATH="$f"
    break
  fi
done

if [[ -z "$ARTIFACT_PATH" ]]; then
  err "No Linux artifact for $APP_NAME $VERSION in $LINUX_OUT_DIR/"
  exit 1
fi

FILENAME="$(basename "$ARTIFACT_PATH")"
FILESIZE=$(stat -f%z "$ARTIFACT_PATH" 2>/dev/null || stat -c%s "$ARTIFACT_PATH")
PLATFORM="linux-x86_64"
ENCLOSURE_URL="${R2_PUBLIC_BASE_URL%/}/releases/$FILENAME"
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")
TITLE="$APP_NAME $VERSION"

info "Uploading $FILENAME to R2..."
npx wrangler r2 object put "$R2_BUCKET/$FILENAME" --file="$ARTIFACT_PATH" --remote
ok "Uploaded: $FILENAME"

# Parent releases row first (release_files.version has an FK to it)
npx wrangler d1 execute "$D1_DATABASE" --remote --command \
  "INSERT OR IGNORE INTO releases (version, short_version_string, title, description, enclosure_url, ed_signature, length, pub_date) VALUES ($BUILD_NUMBER, '$VERSION', '$TITLE', 'Bug fixes and improvements.', '$ENCLOSURE_URL', 'unsigned', $FILESIZE, '$PUB_DATE');"

info "Inserting $PLATFORM into D1 release_files..."
npx wrangler d1 execute "$D1_DATABASE" --remote --command \
  "INSERT OR REPLACE INTO release_files (version, platform, filename, r2_key, size) VALUES ($BUILD_NUMBER, '$PLATFORM', '$FILENAME', '$FILENAME', $FILESIZE);"
ok "D1 record: $PLATFORM"

cd "$PROJECT_DIR"

echo ""
echo -e "  Artifact:  ${BLUE}$FILENAME${NC} ($FILESIZE bytes)"
echo -e "  Platform:  ${BLUE}$PLATFORM${NC}"
echo -e "  Download:  ${BLUE}${R2_PUBLIC_BASE_URL}/latest?platform=linux${NC}"
echo ""
