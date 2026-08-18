#!/bin/bash
# setup-kiosk-macos.sh — Test HMI kiosk mode on macOS
# Usage: bash setup-kiosk-macos.sh [hmi-file-path]

set -e

HMI_FILE="${1:-./hmi-dashboard.html}"
HMI_URL="file://$(pwd)/${HMI_FILE}"

echo "=== Voltrus HMI Kiosk (macOS test) ==="
echo "Opening: ${HMI_URL}"

if [ ! -f "${HMI_FILE}" ]; then
    echo "Error: ${HMI_FILE} not found"
    exit 1
fi

# Open in Safari with minimal chrome
open -a Safari "${HMI_URL}"

echo "For true kiosk mode on macOS:"
echo "  1. Open Safari → Preferences → General"
echo "  2. Set 'Open with' to this HTML file"
echo "  3. System Settings → Displays → disable screen sleep"
echo "  4. Use Amphetamine app to prevent sleep"
