#!/bin/bash
# install-hmi-agent.sh — Install mt-hmi-agent on RPi5/Linux
# Usage: curl -sL https://dl.voltrus.id/hmi-agent/install.sh | sudo bash

set -e

BINARY_NAME="mt-hmi-agent"
INSTALL_DIR="/usr/local/bin"
SERVICE_NAME="mt-hmi-agent"
R2_URL="${R2_URL:-https://dl.voltrus.id/hmi-agent/latest/mt-hmi-agent}"
HMI_DIR="/var/lib/voltrus-hmi"

echo "=== Installing mt-hmi-agent ==="

# Create HMI directory
mkdir -p "${HMI_DIR}"
mkdir -p "${HMI_DIR}/versions"

# Download binary
echo "Downloading ${BINARY_NAME}..."
curl -sL "${R2_URL}" -o "${INSTALL_DIR}/${BINARY_NAME}"
chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

# Create systemd service
cat > "/etc/systemd/system/${SERVICE_NAME}.service" << EOF
[Unit]
Description=Voltrus HMI Deployment Agent
After=network.target

[Service]
Type=simple
ExecStart=${INSTALL_DIR}/${BINARY_NAME}
Restart=always
RestartSec=5
Environment=HMI_DIR=${HMI_DIR}

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "${SERVICE_NAME}"
systemctl restart "${SERVICE_NAME}"

echo ""
echo "=== Installation Complete ==="
echo "Agent running on port 8080"
echo "Deploy HTML:  curl -X POST http://$(hostname -I | awk '{print $1}'):8080/api/deploy -H 'Content-Type: text/html' --data-binary @hmi.html"
echo "Health check: curl http://localhost:8080/api/health"
echo "View HMI:     http://$(hostname -I | awk '{print $1}'):8080/"
