#!/usr/bin/env bash
# Runs the RustRED BACnet flow export (reads, a write node and a COV subscription) in a RustRED release against the
# headless BACnet simulator of mt-bacnet (crates/mt-bacnet/examples/sim_headless.rs) and checks the values that reach the
# debug node: the polled points, the value the write node put into the valve, and a COV notification. Needs gh (or
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
timeout 10 "$RUSTRED" run --headless -u "$work/home" --bind "127.0.0.1:15999" "$work/flow.json" > "$work/run.log" 2>&1 || true

python3 - "$work/run.log" <<'PY'
import math, re, sys
log = open(sys.argv[1]).read()
want = {"Supply temp": 21.5, "Voltage": 230.25, "Fan status": 1, "Mode": 3, "COV true": 21.5}
seen = {}
for key, value in re.findall(r'"(Supply temp|Voltage|Fan status|Mode|COV true)": ([^,\n}]+)', log):
    seen[key] = value.strip()
valves = [float(v) for v in re.findall(r'"Valve": ([^,\n}]+)', log)]
names = re.findall(r'"Supply temp name": "([^"]*)"', log)
bad = []
for key, expected in want.items():
    raw = seen.get(key)
    ok = raw is not None and math.isclose(float(raw), expected, rel_tol=1e-6)
    print(("ok   " if ok else "FAIL ") + key, raw)
    if not ok: bad.append(key)
print(("ok   " if names else "FAIL ") + "Supply temp name", names[:1])
if not names: bad.append("name")
# The valve starts at 50; the write node sets 42 two seconds in, and later polls must read it back.
ok = 50 in valves and valves[-1:] == [42.0]
print(("ok   " if ok else "FAIL ") + "Valve before and after the write", valves[:1], valves[-1:])
if not ok: bad.append("valve")
wrote = '"relinquished": false' in log or 'relinquished' in log
print(("ok   " if wrote else "FAIL ") + "write result reached the debug node")
if not wrote: bad.append("write result")
if bad:
    print(log[-3000:])
    sys.exit(1)
PY
