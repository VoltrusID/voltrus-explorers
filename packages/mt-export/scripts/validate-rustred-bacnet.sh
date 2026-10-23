#!/usr/bin/env bash
# Runs the RustRED BACnet flow export in a RustRED release against the headless BACnet simulator of mt-bacnet
# (crates/mt-bacnet/examples/sim_headless.rs) and checks the values that reach the debug node. Needs gh (or
# RUSTRED=/path/to/rust-red), bun, cargo and python3. SIM=/path/to/sim_headless skips the cargo build.
#
#   bash scripts/validate-rustred-bacnet.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
work=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$work"' EXIT

if [ -z "${RUSTRED:-}" ]; then
  gh release download -R dzulfikar08/rust-red -p 'rust-red-*-linux-x86_64.tar.gz' -D "$work"
  tar -xzf "$work"/rust-red-*-linux-x86_64.tar.gz -C "$work"
  RUSTRED=$(find "$work" -name rust-red -type f -perm -u+x | head -1)
fi
echo "RustRED: $("$RUSTRED" --version 2>&1 | head -1)"

if [ -z "${SIM:-}" ]; then
  (cd "$root" && CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$work/target}" cargo build -q -p mt-bacnet --example sim_headless)
  SIM="${CARGO_TARGET_DIR:-$work/target}/debug/examples/sim_headless"
fi

port=47921
"$SIM" "$port" 1234 > "$work/sim.log" 2>&1 &
sleep 1
bun "$here/dump-bacnet-flow.ts" "$port" > "$work/flow.json"
mkdir -p "$work/home"
timeout 8 "$RUSTRED" run --headless -u "$work/home" --bind "127.0.0.1:15999" "$work/flow.json" > "$work/run.log" 2>&1 || true

python3 - "$work/run.log" <<'PY'
import math, re, sys
log = open(sys.argv[1]).read()
want = {"Supply temp": 21.5, "Voltage": 230.25, "Fan status": 1, "Mode": 3, "Valve": 50}
seen = {}
for key, value in re.findall(r'"(Supply temp|Voltage|Fan status|Mode|Valve)": ([^,\n}]+)', log):
    seen[key] = value.strip()
names = re.findall(r'"Supply temp name": "([^"]*)"', log)
bad = []
for key, expected in want.items():
    raw = seen.get(key)
    ok = raw is not None and math.isclose(float(raw), expected, rel_tol=1e-6)
    print(("ok   " if ok else "FAIL ") + key, raw)
    if not ok: bad.append(key)
print(("ok   " if names else "FAIL ") + "Supply temp name", names[:1])
if not names: bad.append("name")
if bad:
    print(log[-3000:])
    sys.exit(1)
PY
