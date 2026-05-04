#!/usr/bin/env bash
# deploy_rca.sh
#
# Reset the Proxmox test VM to a clean snapshot, build CA packages locally,
# deploy and install them on the guest VM non-interactively.
#
# Usage: ./scripts/deploy_rca.sh

set -euo pipefail

# ---------------------------------------------------------------------------
# Proxmox host
# ---------------------------------------------------------------------------
PROXMOX_HOST="192.168.99.14"
PROXMOX_USER="root"
PROXMOX_PASS="1Password.."
VM_ID=104
SNAPSHOT="Base_Install"

# ---------------------------------------------------------------------------
# Guest VM
# ---------------------------------------------------------------------------
GUEST_HOST="192.168.99.134"
GUEST_USER="justlikeef"
GUEST_PASS="1Password.."

# ---------------------------------------------------------------------------
# CA configuration — written to an answers file on the guest
# ---------------------------------------------------------------------------
CA_HOSTNAME="gagarca01.justlikeef.com"
CA_ORGANIZATION="Justlikeef"
CA_OU="IT Dept"
CA_LOCALITY="Gainesville"
CA_STATE="Georgia"
CA_COUNTRY="US"
CA_PASSPHRASE="1Password.."
CA_CERT_TYPE="1"

# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PKG_DIR="$ROOT_DIR/packages"

SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=5)
SCP_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null)

# ---------------------------------------------------------------------------
# Prerequisites
# ---------------------------------------------------------------------------
if ! command -v sshpass &>/dev/null; then
    echo "ERROR: sshpass is required — install with: sudo apt install sshpass" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
proxmox_ssh() { sshpass -p "$PROXMOX_PASS" ssh "${SSH_OPTS[@]}" "$PROXMOX_USER@$PROXMOX_HOST" "$@"; }
guest_ssh()   { sshpass -p "$GUEST_PASS"   ssh "${SSH_OPTS[@]}" "$GUEST_USER@$GUEST_HOST"     "$@"; }
guest_sudo()  { guest_ssh "echo '${GUEST_PASS}' | sudo -S $*"; }
guest_scp()   { sshpass -p "$GUEST_PASS"   scp "${SCP_OPTS[@]}" "$@"; }

# ---------------------------------------------------------------------------
echo "==> Stopping VM ${VM_ID}..."
proxmox_ssh "qm stop ${VM_ID} 2>/dev/null || true"

echo "==> Rolling back to snapshot '${SNAPSHOT}'..."
proxmox_ssh "qm rollback ${VM_ID} ${SNAPSHOT}"

echo "==> Starting VM ${VM_ID}..."
proxmox_ssh "qm start ${VM_ID}"

# ---------------------------------------------------------------------------
echo "==> Building packages..."
"$SCRIPT_DIR/build_packages_deb.sh"

# ---------------------------------------------------------------------------
echo "==> Waiting for guest to become available..."
BOOT_TIMEOUT=120
ELAPSED=0
until sshpass -p "$GUEST_PASS" ssh "${SSH_OPTS[@]}" -o ConnectTimeout=3 \
      "$GUEST_USER@$GUEST_HOST" true 2>/dev/null; do
    if [[ $ELAPSED -ge $BOOT_TIMEOUT ]]; then
        echo "ERROR: guest unreachable after ${BOOT_TIMEOUT}s" >&2
        exit 1
    fi
    sleep 5
    ELAPSED=$((ELAPSED + 5))
    printf "  ... %ds\n" "$ELAPSED"
done
echo "  Guest is up."

# ---------------------------------------------------------------------------
echo "==> Uploading packages..."
guest_ssh "mkdir -p ~/packages && rm -f ~/packages/*.deb"
guest_scp "$PKG_DIR"/*.deb "$GUEST_USER@$GUEST_HOST:~/packages/"

# ---------------------------------------------------------------------------
echo "==> Writing CA answers file..."
# Written to user home — readable by root during postinst
guest_ssh "cat > ~/ca_answers.env" << EOF
OX_CA_HOSTNAME=${CA_HOSTNAME}
OX_CA_ORGANIZATION=${CA_ORGANIZATION}
OX_CA_OU=${CA_OU}
OX_CA_LOCALITY=${CA_LOCALITY}
OX_CA_STATE=${CA_STATE}
OX_CA_COUNTRY=${CA_COUNTRY}
OX_CA_KEY_PASS=${CA_PASSPHRASE}
OX_CA_CERT_TYPE=${CA_CERT_TYPE}
EOF
guest_ssh "chmod 600 ~/ca_answers.env"

# ---------------------------------------------------------------------------
echo "==> Installing packages..."
guest_ssh "echo '${GUEST_PASS}' | sudo -S env \
  DEBIAN_FRONTEND=noninteractive \
  OX_CA_ANSWERS_FILE=/home/${GUEST_USER}/ca_answers.env \
  apt-get install -y /home/${GUEST_USER}/packages/*.deb"

# ---------------------------------------------------------------------------
echo "==> Cleaning up..."
guest_ssh "rm -f ~/ca_answers.env"

# ---------------------------------------------------------------------------
echo ""
echo "Deploy complete."
echo "  https://${CA_HOSTNAME}/ca/"
