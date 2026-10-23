#!/usr/bin/env bash
# Runs the Node-RED and RustRED flow exports in the latest RustRED release against a local Modbus slave and checks the
# values that come out of the debug node. Needs gh (or RUSTRED=/path/to/rust-red), bun and python3.
#
#   bash scripts/validate-rustred.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
trap 'kill $(jobs -p) 2>/dev/null || true; rm -rf "$work"' EXIT

if [ -z "${RUSTRED:-}" ]; then
  gh release download -R dzulfikar08/rust-red -p 'rust-red-*-linux-x86_64.tar.gz' -D "$work"
  tar -xzf "$work"/rust-red-*-linux-x86_64.tar.gz -C "$work"
  RUSTRED=$(find "$work" -name rust-red -type f -perm -u+x | head -1)
fi
echo "RustRED: $("$RUSTRED" --version 2>&1 | head -1)"

port=15100
for format in rustred nodered; do
  for order in big little swapped; do
    port=$((port + 1))
    python3 "$here/modbus_sim.py" "$port" "$order" &
    sim=$!
    sleep 0.5
    bun "$here/dump-flow.ts" "$format" "$order" "$port" > "$work/$format-$order.json"
    mkdir -p "$work/home-$format-$order"
    timeout 5 "$RUSTRED" run --headless -u "$work/home-$format-$order" --bind "127.0.0.1:$((port + 100))" "$work/$format-$order.json" > "$work/$format-$order.log" 2>&1 || true
    kill "$sim" 2>/dev/null || true
    python3 - "$work/$format-$order.log" "$format-$order" <<'PY'
import math, re, sys
log, name = open(sys.argv[1]).read(), sys.argv[2]
seen = {}
for key, value in re.findall(r'"(Voltage_3000|Voltage_3002|Counter_10|Temperature_20|Energy_100|Flow_200|Run_state_0|Door_5)": ([^,\n]+)', log):
    seen[key] = value
want = {"Voltage_3000": 230.5, "Voltage_3002": 231.25, "Counter_10": 42, "Temperature_20": 21.5, "Energy_100": -70000, "Flow_200": math.pi, "Run_state_0": True, "Door_5": True}
bad = []
for key, expected in want.items():
    raw = seen.get(key)
    got = None if raw is None else (raw == "true" if raw in ("true", "false") else float(raw))
    ok = got == expected if isinstance(expected, bool) else got is not None and math.isclose(got, expected, rel_tol=1e-9)
    if not ok:
        bad.append(f"{key}: expected {expected}, got {raw}")
if bad:
    print(f"FAIL {name}\n  " + "\n  ".join(bad))
    sys.exit(1)
print(f"ok   {name}: {len(want)} values")
PY
  done
done
