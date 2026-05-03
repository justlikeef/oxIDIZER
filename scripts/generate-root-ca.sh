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
ROOT_KEY="$KEY_DIR/ca-root.key"
ROOT_CERT="$CA_DIR/root.crt"

# ---------------------------------------------------------------------------
# Resolve the repo root relative to this script
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
CONTENT_CA_DIR="$REPO_DIR/crates/cert/ox_cert_admin/content/www/ca"
[[ -n "$CONTENT_DIR_OVERRIDE" ]] && CONTENT_CA_DIR="$CONTENT_DIR_OVERRIDE"

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
        --help)
            sed -n '/^# Usage/,/^$/p' "$0" | grep -v '^$'
            exit 0
            ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

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
mkdir -p "$KEY_DIR" "$CA_DIR"
chmod 700 "$KEY_DIR"
chmod 755 "$CA_DIR"
ok "Directories ready: $KEY_DIR, $CA_DIR"

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
    done < <(printf '%s' "$1" | tr ',' '\n')
    printf '%s' "$result"
}
OPENSSL_SUBJECT="$(to_openssl_subj "$SUBJECT")"

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
EOF

# ---------------------------------------------------------------------------
# Passphrase setup — write to temp file to keep it off the command line
# ---------------------------------------------------------------------------
PASSOUT_ARGS=()
PASSIN_ARGS=()
if [[ -n "$PASSPHRASE" ]]; then
    PASS_FILE="$(mktemp /tmp/ox_ca_pass.XXXXXX)"
    chmod 600 "$PASS_FILE"
    printf '%s' "$PASSPHRASE" > "$PASS_FILE"
    CLEANUP_FILES+=("$PASS_FILE")
    PASSOUT_ARGS=(-aes256 -pass "file:$PASS_FILE")
    PASSIN_ARGS=(-passin "file:$PASS_FILE")
fi

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

chmod 400 "$ROOT_KEY"
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
