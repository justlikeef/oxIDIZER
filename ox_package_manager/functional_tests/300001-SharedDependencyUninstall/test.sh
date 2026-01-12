#!/bin/bash

# Configuration
TEST_ID="300001"
TEST_NAME="SharedDependencyUninstall"
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
for pkg in shared_lib app_one app_two; do
    log "Uploading $pkg.tar.gz..."
    response=$(curl -s -X POST -F "package=@${PKGS_DIR}/${pkg}.tar.gz" "${SERVER_URL}/packages/upload/")
    if echo "$response" | grep -q "\"result\":\"success\""; then
        log "Uploaded $pkg successfully."
    else
        log "Failed to upload $pkg: $response"
        exit 1
    fi
done

# 2. Install Packages
# Install shared lib first, then apps
for pkg in shared_lib.tar.gz app_one.tar.gz app_two.tar.gz; do
    log "Installing $pkg..."
    response=$(curl -s -X POST -H "Content-Type: application/json" -d "{\"filename\": \"$pkg\"}" "${SERVER_URL}/packages/install/")
    if echo "$response" | grep -q "\"result\":\"success\""; then
        log "Installed $pkg successfully."
    else
        log "Failed to install $pkg: $response"
        exit 1
    fi
done

# Verify installation
if is_installed "shared_lib" && is_installed "app_one" && is_installed "app_two"; then
    log "All packages installed successfully."
else
    log "Installation verification failed."
    exit 1
fi

# 3. Uninstall Sequence Test
# We are testing backend capability here. The backend does NOT currently enforce blocking uninstalls 
# based on dependencies, so we expect this to SUCCEED.
# The FRONTEND is responsible for the warning/blocking logic for the user.
# This test verifies that the backend operations required for the recursive uninstall (deleting dependants first) works.

# Scenario: Uninstall app_one (Leaf node) -> Should succeed
log "Attempting to uninstall app_one (Leaf node)..."
response=$(curl -s -X POST -H "Content-Type: application/json" -d "{\"package\": \"app_one\"}" "${SERVER_URL}/packages/uninstall")

if echo "$response" | grep -q "\"result\":\"success\""; then
    log "Uninstalled app_one successfully."
else
    log "Failed to uninstall app_one: $response"
    exit 1
fi

if is_installed "app_one"; then
    log "Error: app_one is still installed!"
    exit 1
else
    log "Verified app_one is gone."
fi

# Scenario: Uninstall shared_lib (Dependency of app_two) recursively from a script perspective
# The frontend would typically uninstall app_two first. Let's simulate that sequence.
log "Simulating recursive uninstall: Removing app_two first..."
response=$(curl -s -X POST -H "Content-Type: application/json" -d "{\"package\": \"app_two\"}" "${SERVER_URL}/packages/uninstall")

if echo "$response" | grep -q "\"result\":\"success\""; then
    log "Uninstalled app_two successfully."
else
    log "Failed to uninstall app_two: $response"
    exit 1
fi

log "Now removing shared_lib..."
response=$(curl -s -X POST -H "Content-Type: application/json" -d "{\"package\": \"shared_lib\"}" "${SERVER_URL}/packages/uninstall")

if echo "$response" | grep -q "\"result\":\"success\""; then
    log "Uninstalled shared_lib successfully."
else
    log "Failed to uninstall shared_lib: $response"
    exit 1
fi

# Final Verification
if ! is_installed "shared_lib" && ! is_installed "app_one" && ! is_installed "app_two"; then
    log "Test Passed: Clean uninstall sequence."
    # Cleanup logs on success only if configured (keeping for now)
    exit 0
else
    log "Test Failed: Some packages remain."
    exit 1
fi
