#!/bin/bash
# Refresh crates/focas-proto, the vendored copy of opex-flow's `focas-proto` crate (the FOCAS/Ethernet client,
# CNC simulator, probe and frame capture/replay that CNC Explorer and Voltrus Tools use).
#
#   scripts/sync-focas-proto.sh [--source DIR] [--allow-dirty] [--check]
#
#   --source DIR    the opex-flow checkout (default: $OPEX_FLOW_DIR, else ../../../OpexCG/opex-flow relative to this repo)
#   --allow-dirty   sync even though crates/focas-proto has uncommitted changes in opex-flow; VENDORED then says so and
#                   the content digest is what identifies the copy (commit the source, sync again when you can)
#   --check         change nothing; exit 1 when crates/focas-proto differs from what a sync would write
#
# opex-flow is a private repository, so this repo cannot depend on it by git URL (CI and contributors cannot clone
# it) or by a relative path (the checkout sits outside this repo and does not exist on CI runners). The crate is copied
# in instead and the commit it came from is recorded in crates/focas-proto/VENDORED. Edit the crate in opex-flow
# only; changes made here are overwritten by the next sync (and reported by --check).
#
# The copy keeps opex-flow's code, tests (including the replay fixtures) and README, with three changes that belong to
# this repository: Cargo.toml says `license = "GPL-3.0-or-later"` (the crate's author owns both repositories), the
# README ends with this repository's licence section instead of opex-flow's, and NOTICE's origin paragraph says so.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/crates/focas-proto"
SRC_ROOT="${OPEX_FLOW_DIR:-$ROOT/../../../OpexCG/opex-flow}"
ALLOW_DIRTY=0
CHECK=0

usage() { sed -n '2,12p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 1; }
while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) [[ $# -ge 2 ]] || usage; SRC_ROOT="$2"; shift 2 ;;
    --allow-dirty) ALLOW_DIRTY=1; shift ;;
    --check) CHECK=1; shift ;;
    -h|--help) usage ;;
    *) echo "unknown argument: $1" >&2; usage ;;
  esac
done

SRC="$SRC_ROOT/crates/focas-proto"
[[ -f "$SRC/Cargo.toml" ]] || { echo "no crates/focas-proto/Cargo.toml in '$SRC_ROOT' (set OPEX_FLOW_DIR or pass --source)" >&2; exit 1; }
SRC_ROOT="$(cd "$SRC_ROOT" && pwd)"
SRC="$SRC_ROOT/crates/focas-proto"

COMMIT="$(git -C "$SRC_ROOT" rev-parse HEAD)"
COMMIT_DATE="$(git -C "$SRC_ROOT" show -s --format=%cI HEAD)"
if [[ -n "$(git -C "$SRC_ROOT" status --porcelain --untracked-files=all -- crates/focas-proto)" ]]; then
  if [[ $ALLOW_DIRTY -eq 0 ]]; then
    echo "crates/focas-proto in $SRC_ROOT has uncommitted changes; commit them first, or pass --allow-dirty" >&2
    git -C "$SRC_ROOT" status --short --untracked-files=all -- crates/focas-proto >&2
    exit 1
  fi
  STATE="modified (uncommitted changes on top of the commit above)"
else
  STATE="clean"
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

# ---- build the vendored tree in $STAGE --------------------------------------------------------------------------
for d in src tests examples; do
  [[ -d "$SRC/$d" ]] && cp -R "$SRC/$d" "$STAGE/$d"
done
cp "$SRC/NOTICE" "$STAGE/NOTICE"

# Manifest: opex-flow's, with the licence of this repository.
awk '
  /^# Same terms as opex-flow itself/ { skip = 1 }
  skip && /^#/ { next }
  skip && /^license *=/ { print "license = \"GPL-3.0-or-later\""; skip = 0; next }
  /^license *=/ { print "license = \"GPL-3.0-or-later\""; next }
  { skip = 0; print }
' "$SRC/Cargo.toml" > "$STAGE/Cargo.toml"
grep -q '^license = "GPL-3.0-or-later"$' "$STAGE/Cargo.toml" || { echo "could not set the licence in Cargo.toml" >&2; exit 1; }

# README: everything before opex-flow's "Use from voltrus" and "License" sections, then this repository's.
sed '/^## Use from voltrus/,$d' "$SRC/README.md" > "$STAGE/README.md"
cat >> "$STAGE/README.md" <<'EOF'
## Use in this repository

`crates/focas-proto` is a **vendored copy** of the crate of the same name in opex-flow (the file [`VENDORED`](VENDORED)
records the commit). Do not edit it here: change it in opex-flow and run `scripts/sync-focas-proto.sh`.
CNC Explorer (`apps/cnc-explorer/src-tauri`) and Voltrus Tools (`apps/voltrus-tools/crates/vt-engine`) depend on it by
path. It replaces `mt-focas`, from which it came.

## License

GPL-3.0-or-later, like the rest of this repository (`license` in `Cargo.toml`). The crate was written for this
project; opex-flow carries its own copy under its own terms. The pyfanuc (Unlicense) and pyfocas (MIT) credits above
and in `NOTICE` stay.
EOF

# NOTICE: same credits, this repository's origin line.
sed '/^Origin$/,$d' "$SRC/NOTICE" > "$STAGE/NOTICE"
cat >> "$STAGE/NOTICE" <<'EOF'
Origin
------

The code was written for the voltrus project (crate mt-focas, same author), developed further in opex-flow, and is
vendored back here from there (see VENDORED). In this repository it is licensed GPL-3.0-or-later like the rest of
the workspace.
EOF

DIGEST="$(cd "$SRC" && find src tests examples Cargo.toml README.md NOTICE -type f 2>/dev/null | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -d' ' -f1)"
cat > "$STAGE/VENDORED" <<EOF
Vendored copy of the focas-proto crate of opex-flow (private repository). Generated by scripts/sync-focas-proto.sh;
do not edit the files here, change them in opex-flow and sync again.

source:   opex-flow, crates/focas-proto
commit:   $COMMIT
date:     $COMMIT_DATE
state:    $STATE
digest:   $DIGEST   (sha256 over the sorted sha256sums of src, tests, examples, Cargo.toml, README.md, NOTICE as found in opex-flow)

Changed on the way in: Cargo.toml (licence GPL-3.0-or-later), README.md (licence and "use" sections), NOTICE (origin paragraph).
EOF

# ---- compare or install ----------------------------------------------------------------------------------------
if [[ $CHECK -eq 1 ]]; then
  if [[ -d "$DEST" ]] && diff -r "$STAGE" "$DEST" > /dev/null; then
    echo "crates/focas-proto is in sync with opex-flow $COMMIT ($STATE)"
    exit 0
  fi
  echo "crates/focas-proto differs from opex-flow $COMMIT ($STATE):" >&2
  diff -rq "$STAGE" "$DEST" >&2 || true
  exit 1
fi

mkdir -p "$DEST"
rm -rf "$DEST/src" "$DEST/tests" "$DEST/examples"
cp -R "$STAGE"/. "$DEST"/
echo "crates/focas-proto synced from opex-flow $COMMIT ($STATE)"
