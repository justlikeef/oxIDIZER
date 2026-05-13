#!/usr/bin/env bash
# generate-root-ca.sh
#
# Generates an ECC P-384 root CA key and self-signed certificate, installs them
# to the standard ox_cert system paths, and makes the cert available for download
# via the ox_cert web server.
#
# Usage:
#   sudo ./scripts/generate-root-ca.sh [OPTIONS]
#
# Options:
#   --subject "CN=..."    Distinguished name (default: CN=ox Root CA,O=oxIDIZER,C=US)
#   --days    N           Validity in days            (default: 9131 = ~25 years)
#   --key-type ec|rsa     Key algorithm               (default: ec)
#   --rsa-bits N          RSA key size when --key-type rsa (default: 4096)
#   --no-trust            Skip adding to system trust store
#   --force               Overwrite existing key/cert without prompting
#   --passphrase  P       Encrypt the generated key with passphrase P
#                         (also reads OX_CA_KEY_PASS env var)
#   --content-dir DIR     Override destination for root.crt web copy
#                         (default: <repo>/crates/cert/ox_cert_admin/content/www/ca)
#   --csr-only            Generate a CSR instead of a self-signed certificate
#   --hostname FQDN       FQDN of the CA host; adds DNS SANs to the certificate
#                         (e.g. gagarca01.justlikeef.com → DNS:gagarca01,DNS:gagarca01.justlikeef.com)
#   --help                Show this help message

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
SUBJECT="CN=ox Root CA,O=oxIDIZER,C=US"
VALIDITY_DAYS=9131           # 25 years
KEY_TYPE="ec"
RSA_BITS=4096
ADD_TRUST=true
FORCE=false
PASSPHRASE="${OX_CA_KEY_PASS:-}"   # passphrase for the generated key; also settable via --passphrase
CONTENT_DIR_OVERRIDE=""            # override CONTENT_CA_DIR; set via --content-dir
CSR_ONLY=false                     # generate a CSR instead of a self-signed cert
HOSTNAME=""                        # optional FQDN; used to populate SubjectAltName

# ---------------------------------------------------------------------------
# OS-specific paths
# ---------------------------------------------------------------------------
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        *)       echo "unknown" ;;
    esac
}

OS=$(detect_os)

case "$OS" in
    linux)
        KEY_DIR="/etc/pki/ox_webservice/private"
        CA_DIR="/etc/pki/ox_webservice/ca"
        ;;
    macos)
        KEY_DIR="/etc/ssl/ox_webservice/private"
        CA_DIR="/etc/ssl/ox_webservice/ca"
        ;;
    windows)
        KEY_DIR="C:/ProgramData/ox_webservice/private"
        CA_DIR="C:/ProgramData/ox_webservice/ca"
        ;;
    *)
        echo "ERROR: Unsupported OS: $(uname -s)" >&2
        exit 1
        ;;
esac
ROOT_KEY="$KEY_DIR/default/ca-root.key.pem"
ROOT_CERT="$CA_DIR/root.crt"

# ---------------------------------------------------------------------------
# Resolve the repo root relative to this script
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
CONTENT_CA_DIR="$REPO_DIR/crates/cert/ox_cert_admin/content/www/ca"

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --subject)      SUBJECT="$2";             shift 2 ;;
        --days)         VALIDITY_DAYS="$2";       shift 2 ;;
        --key-type)     KEY_TYPE="$2";            shift 2 ;;
        --rsa-bits)     RSA_BITS="$2";            shift 2 ;;
        --no-trust)     ADD_TRUST=false;          shift   ;;
        --force)        FORCE=true;               shift   ;;
        --passphrase)   PASSPHRASE="$2";          shift 2 ;;
        --content-dir)  CONTENT_DIR_OVERRIDE="$2"; shift 2 ;;
        --csr-only)     CSR_ONLY=true;            shift   ;;
        --hostname)     HOSTNAME="$2";            shift 2 ;;
        --help)
            sed -n '/^# Usage/,/^$/p' "$0" | grep -v '^$'
            exit 0
            ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

[[ -n "$CONTENT_DIR_OVERRIDE" ]] && CONTENT_CA_DIR="$CONTENT_DIR_OVERRIDE"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
info()  { echo "  [INFO]  $*"; }
ok()    { echo "  [OK]    $*"; }
warn()  { echo "  [WARN]  $*"; }
fatal() { echo "  [ERROR] $*" >&2; exit 1; }

check_root() {
    if [[ "$OS" != "windows" && "$EUID" -ne 0 ]]; then
        fatal "This script must be run as root (sudo ./scripts/generate-root-ca.sh)"
    fi
}

require_cmd() {
    command -v "$1" &>/dev/null || fatal "'$1' is required but not installed."
}

# ---------------------------------------------------------------------------
# Pre-flight
# ---------------------------------------------------------------------------
echo
echo "ox Certificate Authority — Root CA Generator"
echo "============================================="
echo

check_root
require_cmd openssl

if [[ "$FORCE" == false && -f "$ROOT_KEY" ]]; then
    warn "Root key already exists at: $ROOT_KEY"
    read -r -p "  Overwrite? [y/N] " confirm
    [[ "${confirm,,}" == "y" ]] || { info "Aborted."; exit 0; }
fi

# ---------------------------------------------------------------------------
# Create directories
# ---------------------------------------------------------------------------
info "Creating directories..."
mkdir -p "$KEY_DIR/default" "$CA_DIR"
chmod 750 "$KEY_DIR"
chmod 770 "$KEY_DIR/default"
chmod 775 "$CA_DIR"
if command -v getent &>/dev/null && getent group ox_webservice &>/dev/null; then
    chown root:ox_webservice "$KEY_DIR" "$KEY_DIR/default" "$CA_DIR"
fi
ok "Directories ready: $KEY_DIR/default, $CA_DIR"

# ---------------------------------------------------------------------------
# OpenSSL extensions config (written to a temp file)
# ---------------------------------------------------------------------------
CLEANUP_FILES=()
cleanup() { rm -f "${CLEANUP_FILES[@]}"; }
trap cleanup EXIT

EXT_CONF="$(mktemp /tmp/ox_cert_root_ext.XXXXXX.cnf)"
CLEANUP_FILES+=("$EXT_CONF")

# Convert comma-separated DN (CN=foo,O=bar,DC=example,DC=com) to the
# OpenSSL /subj format (/CN=foo/O=bar/DC=example/DC=com).
# Using -subj rather than [req_dn] config is the only reliable way to include
# multiple DC= components — the config-file approach silently drops duplicates.
to_openssl_subj() {
    local result=""
    while IFS= read -r part; do
        part="${part#"${part%%[![:space:]]*}"}"   # ltrim
        part="${part%"${part##*[![:space:]]}"}"   # rtrim
        [[ -n "$part" ]] && result="${result}/${part}"
    done < <(printf '%s\n' "$1" | tr ',' '\n')
    printf '%s' "$result"
}
OPENSSL_SUBJECT="$(to_openssl_subj "$SUBJECT")"

# Build SubjectAltName line from --hostname if provided.
# Adds DNS:<short-hostname> and DNS:<fqdn> (only DNS:<fqdn> if no dots).
SAN_LINE=""
if [[ -n "$HOSTNAME" ]]; then
    SHORT_HOST="${HOSTNAME%%.*}"
    if [[ "$SHORT_HOST" != "$HOSTNAME" ]]; then
        SAN_LINE="subjectAltName         = DNS:${SHORT_HOST}, DNS:${HOSTNAME}"
    else
        SAN_LINE="subjectAltName         = DNS:${HOSTNAME}"
    fi
fi

cat > "$EXT_CONF" <<EOF
[req]
distinguished_name = req_dn
x509_extensions    = v3_root_ca
prompt             = no

[req_dn]

[v3_root_ca]
subjectKeyIdentifier   = hash
authorityKeyIdentifier = keyid:always,issuer:always
basicConstraints       = critical,CA:true,pathlen:1
keyUsage               = critical,keyCertSign,cRLSign
${SAN_LINE}
EOF

# ---------------------------------------------------------------------------
# Passphrase — the ox_cert keystore uses its own AES-256-GCM format (not
# OpenSSL PKCS#8 encryption), so the root CA key file must be unencrypted.
# The passphrase is still written to /etc/ox_webservice/env by postinst
# and used by the keystore to protect auto-generated intermediate CA keys.
# ---------------------------------------------------------------------------
PASSOUT_ARGS=()
PASSIN_ARGS=()

# ---------------------------------------------------------------------------
# Generate root key
# ---------------------------------------------------------------------------
info "Generating root CA key ($KEY_TYPE)..."

case "$KEY_TYPE" in
    ec)
        openssl genpkey \
            -algorithm EC \
            -pkeyopt ec_paramgen_curve:P-384 \
            "${PASSOUT_ARGS[@]}" \
            -out "$ROOT_KEY"
        ;;
    rsa)
        openssl genpkey \
            -algorithm RSA \
            -pkeyopt rsa_keygen_bits:"$RSA_BITS" \
            "${PASSOUT_ARGS[@]}" \
            -out "$ROOT_KEY"
        ;;
    *)
        fatal "Unknown --key-type '$KEY_TYPE'. Use 'ec' or 'rsa'."
        ;;
esac

chmod 640 "$ROOT_KEY"
if command -v getent &>/dev/null && getent group ox_webservice &>/dev/null; then
    chown root:ox_webservice "$ROOT_KEY"
fi
ok "Root key written:  $ROOT_KEY"

# ---------------------------------------------------------------------------
# Generate certificate or CSR
# ---------------------------------------------------------------------------
if [[ "$CSR_ONLY" == true ]]; then
    info "Generating certificate signing request..."
    ROOT_CSR="$CA_DIR/root.csr"

    openssl req \
        -new \
        -key    "$ROOT_KEY" \
        "${PASSIN_ARGS[@]}" \
        -subj   "$OPENSSL_SUBJECT" \
        -out    "$ROOT_CSR" \
        -config "$EXT_CONF"

    chmod 440 "$ROOT_CSR"
    ok "CSR written: $ROOT_CSR"
else
    info "Generating self-signed root certificate ($VALIDITY_DAYS days)..."

    openssl req \
        -new \
        -x509 \
        -key    "$ROOT_KEY" \
        "${PASSIN_ARGS[@]}" \
        -subj   "$OPENSSL_SUBJECT" \
        -out    "$ROOT_CERT" \
        -days   "$VALIDITY_DAYS" \
        -config "$EXT_CONF" \
        -extensions v3_root_ca

    chmod 444 "$ROOT_CERT"
    ok "Root cert written: $ROOT_CERT"
fi

# ---------------------------------------------------------------------------
# Display certificate info (self-signed only)
# ---------------------------------------------------------------------------
if [[ "$CSR_ONLY" != true ]]; then
    echo
    echo "Certificate details:"
    openssl x509 -in "$ROOT_CERT" -noout \
        -subject -issuer -dates \
        -fingerprint -sha256 \
        | sed 's/^/  /'
    echo
fi

# ---------------------------------------------------------------------------
# Install to system trust store
# ---------------------------------------------------------------------------
install_trust_linux() {
    if command -v update-ca-certificates &>/dev/null; then
        # Debian / Ubuntu
        cp "$ROOT_CERT" /usr/local/share/ca-certificates/ox-root-ca.crt
        update-ca-certificates
        ok "Trusted (Debian/Ubuntu): /usr/local/share/ca-certificates/ox-root-ca.crt"
    elif command -v update-ca-trust &>/dev/null; then
        # RHEL / CentOS / Fedora
        cp "$ROOT_CERT" /etc/pki/ca-trust/source/anchors/ox-root-ca.crt
        update-ca-trust extract
        ok "Trusted (RHEL/Fedora): /etc/pki/ca-trust/source/anchors/ox-root-ca.crt"
    else
        # Generic fallback
        cp "$ROOT_CERT" /etc/ssl/certs/ox-root-ca.crt
        warn "Generic trust install: /etc/ssl/certs/ox-root-ca.crt"
        warn "Run your distro's trust update command manually."
    fi
}

install_trust_macos() {
    security add-trusted-cert \
        -d \
        -r trustRoot \
        -k /Library/Keychains/System.keychain \
        "$ROOT_CERT"
    ok "Trusted (macOS System Keychain)"
}

if [[ "$CSR_ONLY" != true ]]; then
    if [[ "$ADD_TRUST" == true ]]; then
        info "Adding to system trust store..."
        case "$OS" in
            linux)  install_trust_linux  ;;
            macos)  install_trust_macos  ;;
            windows)
                warn "Windows: run as Administrator in PowerShell:"
                warn "  certutil -addstore 'Root' '$ROOT_CERT'"
                ;;
        esac
    fi

    # Copy cert to web content directory for download
    info "Copying cert to web content directory..."
    mkdir -p "$CONTENT_CA_DIR"
    cp "$ROOT_CERT" "$CONTENT_CA_DIR/root.crt"
    ok "Web download ready: $CONTENT_CA_DIR/root.crt"
    ok "Download URL:       http://<server>:8080/ca/root.crt"

    echo
    echo "─────────────────────────────────────────────────────────────"
    echo " Root CA generated successfully."
    echo
    echo " Key:  $ROOT_KEY"
    echo " Cert: $ROOT_CERT"
    echo
    echo " Start the cert server with:"
    echo "   ./scripts/start_server.sh notice debug ca"
    echo "─────────────────────────────────────────────────────────────"
    echo
else
    echo
    echo "─────────────────────────────────────────────────────────────"
    echo " CSR generated successfully."
    echo
    echo " Key: $ROOT_KEY"
    echo " CSR: $ROOT_CSR"
    echo
    echo " Submit the CSR to your root CA for signing."
    echo " Once you receive the signed certificate, copy it to:"
    echo "   $ROOT_CERT"
    echo "─────────────────────────────────────────────────────────────"
    echo
fi
