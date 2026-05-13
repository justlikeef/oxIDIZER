#!/usr/bin/env bash
# deploy_ica2.sh
#
# Reset the Proxmox test VM to a clean snapshot, build CA packages locally,
# deploy and install them on the guest VM non-interactively as an
# intermediate CA (cert type 2). Generates a CSR and submits it to the
# root CA for signing.
#
# Usage: ./scripts/deploy_ica2.sh [--skip-build]
#
# Options:
#   --skip-build    Skip the package build step (use existing packages/deb/)

set -euo pipefail

# ---------------------------------------------------------------------------
# Proxmox host
# ---------------------------------------------------------------------------
PROXMOX_HOST="192.168.99.14"
PROXMOX_USER="root"
PROXMOX_PASS="1Password.."
VM_ID=106
SNAPSHOT="BASE_INSTALL"

# ---------------------------------------------------------------------------
# Guest VM
# ---------------------------------------------------------------------------
GUEST_HOST="gagaica02.justlikeef.com"
GUEST_USER="justlikeef"
GUEST_PASS="1Password.."

# ---------------------------------------------------------------------------
# CA configuration — written to an answers file on the guest
# ---------------------------------------------------------------------------
CA_HOSTNAME="gagaica02.justlikeef.com"
CA_ORGANIZATION="Justlikeef"
CA_OU="IT-Dept"
CA_LOCALITY="Gainesville"
CA_STATE="Georgia"
CA_COUNTRY="US"
CA_PASSPHRASE="1Password.."
CA_CERT_TYPE="2"
CA_CSR_SERVER="gagarca01.justlikeef.com"


# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PKG_DIR="$ROOT_DIR/packages/deb"

SKIP_BUILD=false
for arg in "$@"; do
    case "$arg" in
        --skip-build) SKIP_BUILD=true ;;
        *) echo "Unknown option: $arg" >&2; exit 1 ;;
    esac
done

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
proxmox_ssh "qm stop ${VM_ID} 2>/dev/null || true"
proxmox_ssh "qm start ${VM_ID}"

# ---------------------------------------------------------------------------
if [[ "$SKIP_BUILD" == true ]]; then
    echo "==> Skipping package build (--skip-build)."
else
    echo "==> Building packages..."
    "$SCRIPT_DIR/build_packages_deb.sh"
fi

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

# Build the list of packages required for the CA server.
# This intentionally excludes modules the CA doesn't use (e.g. ox-webservice-wsgi)
# so their shared libraries are never loaded by the flow builder.
CA_DEBS=(
    ox-webservice_*.deb
    # Cert plugins
    ox-cert-acme_*.deb ox-cert-admin_*.deb ox-cert-ca-init_*.deb
    ox-cert-crl_*.deb  ox-cert-health_*.deb ox-cert-issue_*.deb
    ox-cert-notify_*.deb ox-cert-ocsp_*.deb ox-cert-p12_*.deb
    ox-cert-ra_*.deb   ox-cert-renew_*.deb  ox-cert-revoke_*.deb
    ox-cert-ssh_*.deb  ox-cert-webhook_*.deb
    # Webservice modules
    ox-webservice-errorhandler-jinja2_*.deb ox-webservice-errorhandler-json_*.deb
    ox-webservice-forwarded-for_*.deb ox-webservice-ping_*.deb
    ox-webservice-redirect_*.deb ox-webservice-status_*.deb
    ox-webservice-stream_*.deb ox-webservice-vary-header_*.deb
    # Content
    ox-content-layout-default_*.deb ox-theme-blue_*.deb
    # CA config + meta-package
    ox-ca-config_*.deb ox-ca-server-*.deb
)
for deb in "${CA_DEBS[@]}"; do
    guest_scp "$PKG_DIR"/$deb "$GUEST_USER@$GUEST_HOST:~/packages/"
done

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
OX_CA_CSR_SERVER=${CA_CSR_SERVER}
OX_CA_CSR_SERVER_CACERT=/etc/pki/ox_webservice/ca/parent-root-ca.crt
EOF
guest_ssh "chmod 600 ~/ca_answers.env"

# ---------------------------------------------------------------------------
echo "==> Installing packages..."
guest_ssh "echo '${GUEST_PASS}' | sudo -S env \
  DEBIAN_FRONTEND=noninteractive \
  OX_CA_ANSWERS_FILE=/home/${GUEST_USER}/ca_answers.env \
  apt-get install -y /home/${GUEST_USER}/packages/*.deb 2>&1"

# ---------------------------------------------------------------------------
echo "==> Cleaning up..."
guest_ssh "rm -f ~/ca_answers.env"

# ---------------------------------------------------------------------------
echo ""
echo "Deploy complete."
echo "  https://${CA_HOSTNAME}/ca/"
