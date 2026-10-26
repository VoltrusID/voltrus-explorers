#!/usr/bin/env bash
# Launch the app once per demo scenario and capture its window.
# Usage: scripts/screenshots-macos.sh <app bundle> <output dir>
set -euo pipefail

app="$1"
out="$2"
here="$(cd "$(dirname "$0")" && pwd)"
binary="$app/Contents/MacOS/voltrus-tools"
mkdir -p "$out"

helper="$(mktemp -d)/window-id"
swiftc -O -o "$helper" "$here/window-id.swift"

status=0
for scenario in welcome points trend traffic simulator editor palette light \
  mqtt-points mqtt-trend mqtt-traffic mqtt-inspector mqtt-editor \
  bacnet-points bacnet-trend bacnet-traffic bacnet-discover bacnet-command \
  dnp3-points dnp3-trend dnp3-traffic dnp3-control dnp3-station dnp3-editor \
  s7-points s7-trend s7-traffic s7-plc s7-write s7-editor; do
  "$binary" --demo "$scenario" &
  pid=$!
  id=""
  for _ in $(seq 1 60); do
    if id="$("$helper" "$pid" 2>/dev/null)"; then break; fi
    sleep 0.5
  done
  if [ -z "$id" ]; then
    echo "::warning::$scenario: no window appeared"
    status=1
  else
    # Let sessions connect and data flow: the trend scenario shows a 30 s
    # window, so it waits until the lines span the whole chart.
    case "$scenario" in *trend) sleep 33 ;; *) sleep 6 ;; esac
    screencapture -o -x -l "$id" "$out/$scenario.png"
    echo "captured $scenario ($(sips -g pixelWidth -g pixelHeight "$out/$scenario.png" | tail -2 | awk '{print $2}' | paste -sd x -))"
  fi
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
done
exit "$status"
