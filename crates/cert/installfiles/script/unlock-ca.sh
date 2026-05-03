#!/bin/bash
set -euo pipefail

if [[ "$EUID" -ne 0 ]]; then
    echo "ERROR: Run as root: sudo /usr/share/ox_webservice/unlock-ca.sh" >&2
    exit 1
fi

KEY_FILE="/etc/pki/ox_webservice/private/ca-root.key"

if [[ ! -f "$KEY_FILE" ]]; then
    echo "ERROR: CA private key not found at $KEY_FILE" >&2
    exit 1
fi

# Key has no passphrase — start directly
if openssl pkey -in "$KEY_FILE" -noout 2>/dev/null; then
    echo "CA key has no passphrase — starting service."
    systemctl start ox_webservice
    exit 0
fi

# Key is encrypted — prompt for passphrase
read -rsp "Enter CA key passphrase: " PASS
echo

if ! openssl pkey -in "$KEY_FILE" -passin "pass:$PASS" -noout 2>/dev/null; then
    echo "ERROR: Incorrect passphrase." >&2
    exit 1
fi

echo "Passphrase verified."

# Write to runtime env file — lives in /run, cleared on reboot
mkdir -p /run/ox_webservice
printf 'OX_CA_KEY_PASS=%s\n' "$PASS" > /run/ox_webservice/env
chmod 600 /run/ox_webservice/env
chown root:ox_webservice /run/ox_webservice/env

echo "Starting ox_webservice..."
systemctl start ox_webservice
