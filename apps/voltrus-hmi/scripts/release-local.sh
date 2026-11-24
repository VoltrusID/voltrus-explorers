#!/bin/bash
set -euo pipefail

# ============================================================
# Voltrus HMI — Local Multi-Platform Release
# ============================================================
# Builds macOS ARM64 + Intel DMGs and Windows zip locally,
# then uploads to R2. No GitHub CI needed.
#
# Usage:
#   ./scripts/release-local.sh                          # Build all + upload
#   ./scripts/release-local.sh --skip-upload            # Build only
#   ./scripts/release-local.sh --target aarch64         # macOS ARM64 only
#   ./scripts/release-local.sh --target x86_64          # macOS Intel only
#   ./scripts/release-local.sh --target windows         # Windows x86_64 only
#   ./scripts/release-local.sh --target mac             # Both macOS archs
#   ./scripts/release-local.sh --bump                   # Auto patch-bump version, then build+upload
#   ./scripts/release-local.sh --bump=minor             # minor-bump (or major) instead of patch
#   SKIP_BUILD=1 ./scripts/release-local.sh --skip-build
#
# Prerequisites:
#   1. rustup target add aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-msvc
#   2. cargo install cargo-xwin
#   3. brew install llvm
#   4. pnpm install (in monorepo root)
#   5. cd cloudflare-worker && pnpm install
#
# Env vars:
#   NOTARIZE=1              Enable Apple notarization
#   CODESIGN_IDENTITY=SHA1  Override signing identity
#   R2_PUBLIC_BASE_URL=...  Override R2 public URL
#   SKIP_BUILD=1            Skip cargo tauri build
#   SKIP_UPLOAD=1           Skip R2/D1 upload
#   BUILD_NUMBER=N          Override build number for D1

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MONOREPO_DIR="$(cd "$SCRIPT_DIR/../../.." && pwd)"
WORKER_DIR="$PROJECT_DIR/cloudflare-worker"
BUILD_DIR="$MONOREPO_DIR/target/release/bundle"
APP_NAME="Voltrus HMI"
PNPM_FILTER="voltrus-hmi"
R2_BUCKET="voltrus-hmi-releases"
R2_PUBLIC_BASE_URL="${R2_PUBLIC_BASE_URL:-https://dl-voltrus-hmi.voltrus.id}"
D1_DATABASE="voltrus-hmi-releases-db"
NOTARIZE="${NOTARIZE:-1}"
SKIP_BUILD="${SKIP_BUILD:-0}"
SKIP_UPLOAD="${SKIP_UPLOAD:-0}"
BUILD_NUMBER="${BUILD_NUMBER:-1}"
VERSION=""
DESCRIPTION="Bug fixes and improvements."
TARGET_FILTER="all"
BUMP="${BUMP:-none}"   # none|patch|minor|major — auto-bump version before building

while [[ $# -gt 0 ]]; do
  case "$1" in
    --skip-upload)  SKIP_UPLOAD=1; shift ;;
    --skip-build)   SKIP_BUILD=1;  shift ;;
    --target)       TARGET_FILTER="${2:-all}"; shift 2 ;;
    --notarize)     NOTARIZE=1;    shift ;;
    --description)  DESCRIPTION="${2:-Bug fixes and improvements.}"; shift 2 ;;
    --build-number) BUILD_NUMBER="${2:-1}"; shift 2 ;;
    --bump)         BUMP="patch"; shift ;;
    --bump=*)       BUMP="${1#*=}"; shift ;;
    --)             shift; break ;;
    -*)             err "Unknown option: $1"; exit 1 ;;
    *)
      if [[ -z "$VERSION" ]]; then
        VERSION="$1"
      else
        DESCRIPTION="$1"
      fi
      shift
      ;;
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
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${BLUE}[*]${NC} $1" >&2; }
ok()    { echo -e "${GREEN}[✓]${NC} $1" >&2; }
warn()  { echo -e "${YELLOW}[!]${NC} $1" >&2; }
err()   { echo -e "${RED}[✗]${NC} $1" >&2; }
step()  { echo -e "\n${BLUE}━━━ $1 ━━━${NC}" >&2; }
banner() { echo -e "\n${BOLD}${BLUE}════════════════════════════════════════════════${NC}\n${BOLD}$1${NC}\n${BOLD}${BLUE}════════════════════════════════════════════════${NC}" >&2; }

# Bump a semver string (patch|minor|major) → echo new version
bump_semver() {
  local part="$1" cur="$2" major minor patch
  IFS='.' read -r major minor patch <<< "$cur"
  case "$part" in
    major) major=$((major+1)); minor=0; patch=0 ;;
    minor) minor=$((minor+1)); patch=0 ;;
    patch) patch=$((patch+1)) ;;
    *) err "Invalid --bump part: $part (use patch|minor|major)"; exit 1 ;;
  esac
  echo "${major}.${minor}.${patch}"
}

# Apply version bump: writes new version back to tauri.conf.json + package.json
# (first occurrence only, so package.json root version is updated, not dep versions).
write_version() {
  local v="$1" f
  for f in "$PROJECT_DIR/src-tauri/tauri.conf.json" "$PROJECT_DIR/package.json"; do
    perl -i -pe 'if (/"version":\s*"[^"]*"/ && !$done) { s/"version":\s*"[^"]*"/"version": "'"$v"'"/; $done=1; }' "$f"
  done
}

if [[ "$BUMP" != "none" ]]; then
  VERSION="$(bump_semver "$BUMP" "$VERSION")"
  write_version "$VERSION"
  ok "Bumped version → $VERSION ($BUMP)"
fi

BUILT_FILES=()

# ============================================================
# FUNCTIONS
# ============================================================

codesign_app() {
  local app_path="$1"

  if [[ -z "${CODESIGN_IDENTITY:-}" ]]; then
    CODESIGN_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
      | awk '/Developer ID Application: Dzulfikar/ { print $2; exit }')
    if [[ -z "$CODESIGN_IDENTITY" ]]; then
      CODESIGN_IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null \
        | awk '/Apple Development:/ { print $2; exit }')
      if [[ -n "$CODESIGN_IDENTITY" ]]; then
        warn "Using Apple Development certificate (not for distribution)"
      fi
    fi
    if [[ -z "$CODESIGN_IDENTITY" ]]; then
      err "No signing identity found. Set CODESIGN_IDENTITY."
      exit 1
    fi
  fi

  info "Signing with: $CODESIGN_IDENTITY"
  codesign --deep --force --verbose \
    --sign "$CODESIGN_IDENTITY" \
    --options runtime \
    --entitlements "$PROJECT_DIR/src-tauri/entitlements.plist" \
    "$app_path" 2>&1 || {
    err "Codesign failed"
    return 1
  }
  codesign --verify --verbose "$app_path" 2>&1 || warn "Verify warning (non-fatal)"
  ok "Signed: $(basename "$app_path")"
}

SIGNING_KEY_PATH="${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/.config/tauri/voltrus-hmi.key}"
SIGNING_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

sign_update_file() {
  local filepath="$1"

  if [[ ! -f "$SIGNING_KEY_PATH" ]]; then
    warn "No update signing key at $SIGNING_KEY_PATH — skipping signature"
    echo ""
    return
  fi

  info "Signing update file: $(basename "$filepath")"
  local sig
  sig=$(cargo tauri signer sign -f "$SIGNING_KEY_PATH" ${SIGNING_KEY_PASSWORD:+-p "$SIGNING_KEY_PASSWORD"} "$filepath" 2>&1 | sed -n '/^Public signature:$/,/^$/{ /^Public signature:$/d; /^Make sure/d; p; }' | tr -d '\n')

  if [[ -n "$sig" && "$sig" != *"ERR"* && "$sig" != *"error"* && "$sig" != *"Error"* ]]; then
    ok "Update signed"
    echo "$sig"
  else
    warn "Signature generation failed: $sig"
    echo ""
  fi
}

notarize_app() {
  local app_path="$1"
  local version="$2"

  CERT_NAME=$(security find-identity -v -p codesigning 2>/dev/null | grep "$CODESIGN_IDENTITY" | head -1)
  if [[ "$CERT_NAME" =~ "Apple Development" ]]; then
    warn "Skipping notarization (development certificate)"
    return
  fi

  if [[ "$NOTARIZE" != "1" ]]; then
    warn "Skipping notarization (set NOTARIZE=1 to enable)"
    return
  fi

  local NOTARY_PROFILE="${NOTARY_KEYCHAIN_PROFILE:-waqtu-notary}"
  local zip_path="$BUILD_DIR/$(basename "$app_path" .app)-$version.zip"
  rm -f "$zip_path"
  ditto -c -k --keepParent "$app_path" "$zip_path"

  info "Submitting to Apple notarization..."
  xcrun notarytool submit "$zip_path" --keychain-profile "$NOTARY_PROFILE" --wait

  info "Stapling notarization ticket..."
  xcrun stapler staple "$app_path"
  ok "Notarized and stapled"
}

create_dmg() {
  local app_path="$1"
  local arch="$2"
  local dmg_name="${APP_NAME// /_}_${VERSION}_${arch}.dmg"
  local dmg_path="$BUILD_DIR/dmg/$dmg_name"

  info "Creating DMG from signed/notarized app..."
  local staging="$BUILD_DIR/dmg-staging-$arch"
  rm -rf "$staging"
  mkdir -p "$staging"
  cp -R "$app_path" "$staging/"
  ln -sf /Applications "$staging/Applications"

  mkdir -p "$(dirname "$dmg_path")"
  rm -f "$dmg_path"
  hdiutil create \
    -volname "$APP_NAME $VERSION" \
    -srcfolder "$staging" \
    -ov -format UDZO \
    "$dmg_path" >&2

  rm -rf "$staging"

  local file_size
  file_size=$(stat -f%z "$dmg_path" 2>/dev/null || stat -c%s "$dmg_path" 2>/dev/null || echo 0)
  local file_size_mb
  file_size_mb=$(echo "scale=1; $file_size / 1048576" | bc)
  ok "DMG: $dmg_path (${file_size_mb} MB)"

  echo "$dmg_path"
}

build_macos() {
  local arch="$1"
  local target="${arch}-apple-darwin"
  local platform_key="darwin-${arch}"

  step "macOS $arch ($target)"

  if [[ "$SKIP_BUILD" == "1" ]]; then
    local found_app=""
    for candidate in \
      "$MONOREPO_DIR/target/$target/release/bundle/macos/$APP_NAME.app" \
      "$BUILD_DIR/macos/$APP_NAME.app" \
      "$BUILD_DIR/macos-$arch/$APP_NAME.app"; do
      if [[ -d "$candidate" ]]; then
        found_app="$candidate"
        break
      fi
    done
    if [[ -n "$found_app" ]]; then
      warn "Skipping build, using existing: $found_app"
      echo "$found_app"
      return
    fi
    err "App not found — cannot skip build"
    return 1
  fi

  info "Building Tauri app for $target..."
  cd "$PROJECT_DIR"

  pnpm --filter "$PNPM_FILTER" build 2>&1 | tail -5 >&2

  if [[ "$arch" == "aarch64" ]]; then
    pnpm tauri build 2>&1 | tail -20 >&2
  else
    local sdk_path
    sdk_path=$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)
    if [[ -n "$sdk_path" ]]; then
      SDKROOT="$sdk_path" pnpm tauri build --target "$target" 2>&1 | tail -20 >&2
    else
      pnpm tauri build --target "$target" 2>&1 | tail -20 >&2
    fi
  fi

  local app_path=""
  for candidate in \
    "$MONOREPO_DIR/target/$target/release/bundle/macos/$APP_NAME.app" \
    "$BUILD_DIR/macos/$APP_NAME.app" \
    "$BUILD_DIR/macos-$arch/$APP_NAME.app"; do
    if [[ -d "$candidate" ]]; then
      app_path="$candidate"
      break
    fi
  done

  if [[ -z "$app_path" ]]; then
    err "Build failed — app not found for $arch"
    return 1
  fi

  ok "Built: $app_path"
  echo "$app_path"
}

build_windows() {
  local target="x86_64-pc-windows-msvc"
  local platform_key="windows-x86_64"
  local win_build_dir="$MONOREPO_DIR/target/$target/release/bundle"

  step "Windows x86_64 ($target)"

  if ! command -v cargo-xwin &>/dev/null; then
    err "cargo-xwin not found. Install with: cargo install cargo-xwin"
    return 1
  fi

  local llvm_rc=""
  for candidate in \
    /opt/homebrew/opt/llvm/bin/llvm-rc \
    /usr/local/opt/llvm/bin/llvm-rc \
    "$(brew --prefix llvm 2>/dev/null)/bin/llvm-rc"; do
    if [[ -x "$candidate" ]]; then
      llvm_rc="$candidate"
      break
    fi
  done
  # Linux hosts: llvm-rc on PATH, or a versioned llvm-rc-NN (Debian/Ubuntu)
  if [[ -z "$llvm_rc" ]]; then
    llvm_rc="$(command -v llvm-rc 2>/dev/null || true)"
  fi
  if [[ -z "$llvm_rc" ]]; then
    local llvm_rc_versioned
    llvm_rc_versioned="$(compgen -c llvm-rc- 2>/dev/null | sort -V | tail -1 || true)"
    [[ -n "$llvm_rc_versioned" ]] && llvm_rc="$(command -v "$llvm_rc_versioned" 2>/dev/null || true)"
  fi
  if [[ -z "$llvm_rc" ]]; then
    err "llvm-rc not found. Install with: brew install llvm (macOS) or your distro's llvm package (Linux)"
    return 1
  fi
  ok "Using llvm-rc: $llvm_rc"

  eval "$(cargo xwin env --target "$target" 2>/dev/null)"
  local llvm_bin
  llvm_bin="$(dirname "$llvm_rc")"
  export PATH="$llvm_bin:$PATH"

  if [[ "$SKIP_BUILD" == "1" ]]; then
    local found=""
    local want="${APP_NAME// /_}"
    for pattern in \
      "$win_build_dir/nsis/"*.exe \
      "$win_build_dir/msi/"*.msi \
      "$win_build_dir/"*.zip \
      "$MONOREPO_DIR/target/$target/release/${want}_*.zip"; do
      if [[ -f "$pattern" ]]; then
        local base; base="$(basename "$pattern")"
        # Only accept THIS app's artifact (shared monorepo bundle dir has other apps)
        if [[ "$base" == *"$VERSION"* ]] && { [[ "$base" == *"$APP_NAME"* ]] || [[ "$base" == *"$want"* ]]; }; then
          found="$pattern"
          break
        fi
      fi
    done
    if [[ -n "$found" ]]; then
      warn "Skipping build, using existing: $found"
      echo "$found"
      return
    fi
    err "Windows artifact not found — cannot skip build"
    return 1
  fi

  info "Building Tauri app for $target..."
  cd "$PROJECT_DIR"

  pnpm --filter "$PNPM_FILTER" build 2>&1 | tail -5 >&2

  pnpm tauri build --target "$target" 2>&1 | tail -30 >&2 || true

  local exe_path="$MONOREPO_DIR/target/$target/release/voltrus_hmi.exe"

  local installer_path=""
  for candidate in \
    "$win_build_dir/nsis/"*.exe \
    "$win_build_dir/msi/"*.msi; do
    if [[ -f "$candidate" ]]; then
      local base; base="$(basename "$candidate")"
      # Only accept THIS app's installer (shared monorepo bundle dir has other apps)
      if [[ "$base" == *"$VERSION"* && "$base" == *"$APP_NAME"* ]]; then
        installer_path="$candidate"
        break
      fi
    fi
  done

  if [[ -n "$installer_path" ]]; then
    local file_size
    file_size=$(stat -f%z "$installer_path" 2>/dev/null || stat -c%s "$installer_path" 2>/dev/null || echo 0)
    local file_size_mb
    file_size_mb=$(echo "scale=1; $file_size / 1048576" | bc)
    ok "Built installer: $installer_path (${file_size_mb} MB)"
    echo "$installer_path"
    return
  fi

  if [[ ! -f "$exe_path" ]]; then
    err "Build failed — exe not found at $exe_path"
    return 1
  fi

  warn "NSIS not available, creating .zip from Tauri-built exe..."
  local zip_name="${APP_NAME// /_}_${VERSION}_x64.zip"
  local zip_dir="$win_build_dir"
  mkdir -p "$zip_dir"
  local zip_path="$zip_dir/$zip_name"
  rm -f "$zip_path"

  cd "$(dirname "$exe_path")"
  zip -j "$zip_path" "$(basename "$exe_path")" >&2
  cd "$PROJECT_DIR"

  if [[ ! -f "$zip_path" ]]; then
    err "Failed to create zip at $zip_path"
    return 1
  fi

  local file_size
  file_size=$(stat -f%z "$zip_path" 2>/dev/null || stat -c%s "$zip_path" 2>/dev/null || echo 0)
  local file_size_mb
  file_size_mb=$(echo "scale=1; $file_size / 1048576" | bc)
  ok "Built zip: $zip_path (${file_size_mb} MB)"

  echo "$zip_path"
}

# ============================================================
# RESOLVE PLATFORMS
# ============================================================

PLATFORMS=()

case "$TARGET_FILTER" in
  all)       PLATFORMS=("aarch64|aarch64-apple-darwin|darwin-aarch64|macos" "x86_64|x86_64-apple-darwin|darwin-x86_64|macos" "windows|x86_64-pc-windows-msvc|windows-x86_64|windows") ;;
  mac)       PLATFORMS=("aarch64|aarch64-apple-darwin|darwin-aarch64|macos" "x86_64|x86_64-apple-darwin|darwin-x86_64|macos") ;;
  mac-arm)   PLATFORMS=("aarch64|aarch64-apple-darwin|darwin-aarch64|macos") ;;
  mac-intel) PLATFORMS=("x86_64|x86_64-apple-darwin|darwin-x86_64|macos") ;;
  aarch64)   PLATFORMS=("aarch64|aarch64-apple-darwin|darwin-aarch64|macos") ;;
  x86_64)    PLATFORMS=("x86_64|x86_64-apple-darwin|darwin-x86_64|macos") ;;
  windows)   PLATFORMS=("windows|x86_64-pc-windows-msvc|windows-x86_64|windows") ;;
  win)       PLATFORMS=("windows|x86_64-pc-windows-msvc|windows-x86_64|windows") ;;
  *)         err "Unknown target: $TARGET_FILTER (use: all, mac, mac-arm, mac-intel, windows)"; exit 1 ;;
esac

# ============================================================
# MAIN
# ============================================================

banner "Voltrus HMI $VERSION — Local Release"

info "Platforms: $(for p in "${PLATFORMS[@]}"; do IFS='|' read -r lbl _ _ _ <<< "$p"; echo -n "$lbl "; done)"

for p in "${PLATFORMS[@]}"; do
  IFS='|' read -r _ target _ _ <<< "$p"
  if [[ "$target" == *"-apple-darwin" ]]; then
    if ! rustup target list --installed | grep -q "$target"; then
      warn "Installing missing target: $target"
      rustup target add "$target"
    fi
  fi
done
ok "Rust targets ready"

if [[ "$SKIP_UPLOAD" != "1" ]]; then
  if [[ -d "$WORKER_DIR" ]]; then
    if [[ ! -d "$WORKER_DIR/node_modules" ]]; then
      info "Installing wrangler dependencies..."
      cd "$WORKER_DIR" && npm install && cd "$PROJECT_DIR"
    fi
    ok "Wrangler ready"
  else
    warn "No cloudflare-worker/ directory — skipping upload"
    SKIP_UPLOAD=1
  fi
fi

# ============================================================
# BUILD EACH PLATFORM
# ============================================================

for p in "${PLATFORMS[@]}"; do
  IFS='|' read -r label target platform_key os_type <<< "$p"

  if [[ "$os_type" == "macos" ]]; then
    app_path=$(build_macos "$label") || {
      err "Failed to build macOS $label, skipping..."
      continue
    }

    codesign_app "$app_path"
    notarize_app "$app_path" "$VERSION"

    dmg_path=$(create_dmg "$app_path" "$label")
    BUILT_FILES+=("$label|$platform_key|$(basename "$dmg_path")|$dmg_path|$app_path")

  elif [[ "$os_type" == "windows" ]]; then
    installer_path=$(build_windows) || {
      err "Failed to build Windows, skipping..."
      continue
    }

    local_filename="$(basename "$installer_path")"
    normalized_filename="${local_filename// /_}"
    if [[ "$normalized_filename" != "$local_filename" ]]; then
      dest_dir="$(dirname "$installer_path")"
      cp "$installer_path" "$dest_dir/$normalized_filename"
      installer_path="$dest_dir/$normalized_filename"
    fi

    BUILT_FILES+=("$label|$platform_key|$normalized_filename|$installer_path|")
  fi
done

# ============================================================
# UPLOAD TO R2 & D1
# ============================================================

if [[ "$SKIP_UPLOAD" == "1" ]]; then
  banner "Build Complete (upload skipped)"
  if [[ ${#BUILT_FILES[@]} -gt 0 ]]; then
    for entry in "${BUILT_FILES[@]}"; do
      IFS='|' read -r label platform_key filename filepath _ <<< "$entry"
      echo -e "  ${GREEN}$label${NC} (${platform_key}): ${BLUE}$filepath${NC}"
    done
  else
    warn "No platforms were built successfully"
  fi
  echo ""
  echo -e "  ${YELLOW}To upload:${NC}  SKIP_BUILD=1 ./scripts/release-local.sh --skip-build"
  echo ""
  exit 0
fi

step "Uploading to R2 & D1"

cd "$WORKER_DIR"

for entry in "${BUILT_FILES[@]:-}"; do
  [[ -z "$entry" ]] && continue
  IFS='|' read -r label platform_key filename filepath app_path <<< "$entry"

  info "Uploading $filename to R2..."
  npx wrangler r2 object put "$R2_BUCKET/$filename" --file="$filepath" --remote
  ok "Uploaded: $filename"

  filepath_size=$(stat -f%z "$filepath" 2>/dev/null || stat -c%s "$filepath" 2>/dev/null || echo 0)

  update_signature=""
  update_r2_key=""
  update_size=0
  if [[ "$os_type" == "macos" ]] || [[ "$platform_key" == darwin-* ]]; then
    if [[ -n "$app_path" && -d "$app_path" ]]; then
      update_tarball="$BUILD_DIR/${APP_NAME// /_}_${VERSION}_${platform_key}.app.tar.gz"
      info "Creating update tarball for $platform_key..."
      rm -f "$update_tarball"
      tar -czf "$update_tarball" -C "$(dirname "$app_path")" "$(basename "$app_path")"
      update_r2_key="$(basename "$update_tarball")"
      update_size=$(stat -f%z "$update_tarball" 2>/dev/null || stat -c%s "$update_tarball" 2>/dev/null || echo 0)

      update_signature=$(sign_update_file "$update_tarball")

      info "Uploading update tarball $update_r2_key to R2..."
      npx wrangler r2 object put "$R2_BUCKET/$update_r2_key" --file="$update_tarball" --remote
      ok "Uploaded: $update_r2_key"
    fi
  fi

  if [[ "$platform_key" == windows-* ]]; then
    update_r2_key="$filename"
    update_size=$filepath_size
    update_signature=$(sign_update_file "$filepath")
  fi

  sig_esc="${update_signature//\'/\'\'}"

  info "Inserting $platform_key into D1 release_files..."
  npx wrangler d1 execute "$D1_DATABASE" --remote --command \
    "INSERT OR REPLACE INTO release_files (version, platform, filename, r2_key, size, signature) VALUES ($BUILD_NUMBER, '$platform_key', '$filename', '$filename', $filepath_size, '$sig_esc');"
  ok "D1 record: $platform_key"

  if [[ -n "$update_r2_key" && "$update_r2_key" != "$filename" ]]; then
    info "Inserting update tarball record for $platform_key..."
    npx wrangler d1 execute "$D1_DATABASE" --remote --command \
      "INSERT OR REPLACE INTO release_files (version, platform, filename, r2_key, size, signature) VALUES ($BUILD_NUMBER, '${platform_key}-update', '$update_r2_key', '$update_r2_key', $update_size, '$sig_esc');"
    ok "D1 update record: ${platform_key}-update"
  fi
done

if [[ ${#BUILT_FILES[@]} -gt 0 ]]; then
  first_entry="${BUILT_FILES[0]}"
  IFS='|' read -r _ _ filename filepath _ <<< "$first_entry"
  filepath_size=$(stat -f%z "$filepath" 2>/dev/null || stat -c%s "$filepath" 2>/dev/null || echo 0)
  enclosure_url="${R2_PUBLIC_BASE_URL%/}/releases/$filename"
  pub_date=$(date -u +"%Y-%m-%dT%H:%M:%S.000Z")
  title="$APP_NAME $VERSION"
  title_esc="${title//\'/\'\'}"
  desc_esc="${DESCRIPTION//\'/\'\'}"

  info "Updating legacy releases table for appcast..."
  npx wrangler d1 execute "$D1_DATABASE" --remote --command \
    "INSERT OR REPLACE INTO releases (version, short_version_string, title, description, enclosure_url, ed_signature, length, pub_date) VALUES ($BUILD_NUMBER, '$VERSION', '$title_esc', '$desc_esc', '$enclosure_url', 'unsigned', $filepath_size, '$pub_date');"
  ok "Appcast record updated"
fi

cd "$PROJECT_DIR"

# ============================================================
# DONE
# ============================================================

banner "Release Complete — v$VERSION"

for entry in "${BUILT_FILES[@]:-}"; do
  [[ -z "$entry" ]] && continue
  IFS='|' read -r label platform_key filename filepath _ <<< "$entry"
  file_size=$(stat -f%z "$filepath" 2>/dev/null || stat -c%s "$filepath" 2>/dev/null || echo 0)
  file_size_mb=$(echo "scale=1; $file_size / 1048576" | bc)
  echo -e "  ${GREEN}$label${NC} (${platform_key}): ${BLUE}$filepath${NC} (${file_size_mb} MB)"
done

echo ""
echo -e "  Download:  ${BLUE}${R2_PUBLIC_BASE_URL}/latest${NC}"
echo -e "  Appcast:   ${BLUE}${R2_PUBLIC_BASE_URL}/appcast.xml${NC}"
echo -e "  Downloads: ${BLUE}${R2_PUBLIC_BASE_URL}/downloads${NC}"
echo ""

if [[ "$NOTARIZE" == "0" ]]; then
  echo -e "  ${YELLOW}Tip: Notarization was skipped. Set NOTARIZE=1 for production macOS distribution${NC}"
fi

echo -e "  ${YELLOW}Deploy worker if updated:${NC}"
echo -e "    cd $WORKER_DIR && npx wrangler deploy"
echo ""
