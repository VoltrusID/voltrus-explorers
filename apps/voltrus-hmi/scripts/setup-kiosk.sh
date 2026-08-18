#!/bin/bash
# setup-kiosk.sh — Configure RPi5/Debian for HMI kiosk mode
# Usage: sudo bash setup-kiosk.sh [hmi-url]
# Example: sudo bash setup-kiosk.sh http://192.168.1.100:8080

set -e

HMI_URL="${1:-http://localhost:8080}"
SERVICE_NAME="voltrus-hmi-kiosk"
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
AUTOSTART_DIR="/home/pi/.config/lxsession/LXDE-pi"
AUTOSTART_FILE="${AUTOSTART_DIR}/autostart"

echo "=== Voltrus HMI Kiosk Setup ==="
echo "HMI URL: ${HMI_URL}"

# Install Chromium if missing
if ! command -v chromium-browser &> /dev/null && ! command -v chromium &> /dev/null; then
    echo "Installing Chromium..."
    apt-get update -qq
    apt-get install -y -qq chromium-browser 2>/dev/null || apt-get install -y -qq chromium
fi

CHROMIUM_BIN=$(command -v chromium-browser 2>/dev/null || command -v chromium 2>/dev/null)

# Install unclutter to hide mouse cursor
if ! command -v unclutter &> /dev/null; then
    echo "Installing unclutter (mouse hider)..."
    apt-get install -y -qq unclutter
fi

# Disable screen blanking
echo "Disabling screen blanking..."
xset s off 2>/dev/null || true
xset -dpms 2>/dev/null || true
xset s noblank 2>/dev/null || true

# Make screen blanking persistent
grep -q "xset s off" /etc/xdg/lxsession/LXDE-pi/autostart 2>/dev/null || {
    sed -i '/@xscreensaver/d' /etc/xdg/lxsession/LXDE-pi/autostart 2>/dev/null || true
    cat >> /etc/xdg/lxsession/LXDE-pi/autostart << 'XSET'
@xset s off
@xset -dpms
@xset s noblank
@unclutter -idle 0.1 -root
XSET
}

# Create systemd service for kiosk mode
echo "Creating systemd service..."
cat > "${SERVICE_FILE}" << EOF
[Unit]
Description=Voltrus HMI Kiosk
After=network.target

[Service]
Type=simple
User=pi
Environment=DISPLAY=:0
ExecStartPre=/bin/sleep 5
ExecStart=${CHROMIUM_BIN} --kiosk --app=${HMI_URL} --no-first-run --disable-infobars --disable-session-crashed-bubble --disable-translate --noerrdialogs
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "${SERVICE_NAME}"
echo ""
echo "=== Setup Complete ==="
echo "Start kiosk:  sudo systemctl start ${SERVICE_NAME}"
echo "Stop kiosk:   sudo systemctl stop ${SERVICE_NAME}"
echo "View log:     journalctl -u ${SERVICE_NAME} -f"
echo "Auto-starts on boot."
