#!/bin/bash
set -euo pipefail

KEY_FILE="/etc/pki/ox_webservice/private/ca-root.key"

if [[ ! -f "$KEY_FILE" ]]; then
    echo "ERROR: CA private key not found at $KEY_FILE" >&2
    exit 1
fi

PASS="${OX_CA_KEY_PASS:-}"

if [[ -n "$PASS" ]]; then
    if openssl pkey -in "$KEY_FILE" -passin "pass:$PASS" -noout 2>/dev/null; then
        echo "CA key passphrase verified."
        exit 0
    else
        echo "ERROR: OX_CA_KEY_PASS does not unlock the CA key." >&2
        echo "       Run: sudo /usr/share/ox_webservice/unlock-ca.sh" >&2
        exit 1
    fi
else
    if openssl pkey -in "$KEY_FILE" -noout 2>/dev/null; then
        echo "CA key has no passphrase — proceeding."
        exit 0
    else
        echo "ERROR: CA key requires a passphrase but OX_CA_KEY_PASS is not set." >&2
        echo "       Run: sudo /usr/share/ox_webservice/unlock-ca.sh" >&2
        exit 1
    fi
fi
