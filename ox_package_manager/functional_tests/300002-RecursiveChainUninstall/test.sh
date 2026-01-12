#!/bin/bash

# Configuration
TEST_ID="300002"
TEST_NAME="RecursiveChainUninstall"
SERVER_URL="http://127.0.0.1:3000"
PKGS_DIR="/var/repos/oxIDIZER/ox_package_manager/test_pkgs"
LOG_DIR="/var/repos/oxIDIZER/ox_package_manager/functional_tests/${TEST_ID}-${TEST_NAME}/logs"

# Ensure log directory exists
mkdir -p "$LOG_DIR"
LOG_FILE="${LOG_DIR}/test.log"

# Function to log messages
log() {
    echo "$(date '+%Y-%m-%d %H:%M:%S') - $1" | tee -a "$LOG_FILE"
}

# Function to check if a package is installed
is_installed() {
    local pkg_name=$1
    curl -s "${SERVER_URL}/packages/list/installed" | grep -q "$pkg_name"
    return $?
}

log "Starting Test ${TEST_ID}: ${TEST_NAME}"

# 1. Upload Packages
log "Uploading packages..."
for pkg in chain_base chain_mid chain_top; do
    log "Uploading $pkg.tar.gz..."
    response=$(curl -s -X POST -F "package=@${PKGS_DIR}/${pkg}.tar.gz" "${SERVER_URL}/packages/upload/")
    if echo "$response" | grep -q "\"result\":\"success\""; then
        log "Uploaded $pkg successfully."
    else
        log "Failed to upload $pkg: $response"
        exit 1
    fi
done

# 2. Install Packages order: Base -> Mid -> Top
for pkg in chain_base.tar.gz chain_mid.tar.gz chain_top.tar.gz; do
    log "Installing $pkg..."
    response=$(curl -s -X POST -H "Content-Type: application/json" -d "{\"filename\": \"$pkg\"}" "${SERVER_URL}/packages/install/")
    if echo "$response" | grep -q "\"result\":\"success\""; then
        log "Installed $pkg successfully."
    else
        log "Failed to install $pkg: $response"
        exit 1
    fi
done

# Verify
if is_installed "chain_base" && is_installed "chain_mid" && is_installed "chain_top"; then
    log "Chain packages installed."
else
    log "Installation failure."
    exit 1
fi

# 3. Simulate Recursive Uninstall (Leaf first)
# The frontend detects dependency: Top depends on Mid depends on Base.
# If user uninstalls Base, frontend calculates: [Top, Mid, Base] to remove.

log "Uninstalling chain_top..."
curl -s -X POST -H "Content-Type: application/json" -d "{\"package\": \"chain_top\"}" "${SERVER_URL}/packages/uninstall" > /dev/null

log "Uninstalling chain_mid..."
curl -s -X POST -H "Content-Type: application/json" -d "{\"package\": \"chain_mid\"}" "${SERVER_URL}/packages/uninstall" > /dev/null

log "Uninstalling chain_base..."
curl -s -X POST -H "Content-Type: application/json" -d "{\"package\": \"chain_base\"}" "${SERVER_URL}/packages/uninstall" > /dev/null

# Verify
if ! is_installed "chain_base" && ! is_installed "chain_mid" && ! is_installed "chain_top"; then
    log "Test Passed: Chain completely removed."
    exit 0
else
    log "Test Failed: Debris remains."
    exit 1
fi
