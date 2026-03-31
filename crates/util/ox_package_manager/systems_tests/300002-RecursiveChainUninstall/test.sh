#!/bin/bash

SCRIPTS_DIR=$1
TEST_LIBS_DIR=$2
MODE=$3
LOGGING_LEVEL=$4
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

source "$TEST_LIBS_DIR/log_function.sh"
TEST_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"
PKGS_DIR="$WORKSPACE_DIR/crates/util/ox_package_manager/test_pkgs"

if [ "$MODE" == "integrated" ]; then
    log_message "$LOGGING_LEVEL" "info" "Skipping in integrated mode"
    exit 77
fi

log_message "$LOGGING_LEVEL" "info" "Starting Test: 300002-RecursiveChainUninstall"

TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
LOG_FILE="$TEST_DIR/logs/ox_webservice.log"
STAGING_DIR="$TEST_DIR/staging"
mkdir -p "$TEST_DIR/logs" "$TEST_DIR/conf" "$STAGING_DIR"

cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: package_manager
    name: ox_package_manager
    path: "$WORKSPACE_DIR/target/$TARGET/libox_package_manager.so"
    staging_directory: "$STAGING_DIR"

servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "0.0.0.0"
    hosts:
      - name: "localhost"

workflow:
  name: "ox_webservice"
  stages:
    - name: Content
      runner: sequential
      plugins:
        - name: ox_webservice_router
      on_error: continue

routes:
  - url: "^/packages/(upload|list|install|uninstall)/?"
    module_id: "package_manager"
    priority: 450
    headers:
      Method: POST
  - url: "^/packages/(list)/?"
    module_id: "package_manager"
    priority: 450
EOF

SERVER_URL="http://127.0.0.1:$BASE_PORT"

"$SCRIPTS_DIR/start_server.sh" "$LOGGING_LEVEL" "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$LOG_FILE" "$TEST_PID_FILE" "$WORKSPACE_DIR"
sleep 5

if [ ! -f "$TEST_PID_FILE" ] || ! kill -0 "$(cat "$TEST_PID_FILE")" 2>/dev/null; then
    log_message "$LOGGING_LEVEL" "error" "Server failed to start"
    cat "$LOG_FILE" 2>/dev/null || true
    exit 1
fi

FAILURES=0

is_installed() {
    curl -s "${SERVER_URL}/packages/list/installed" | grep -q "$1"
}

# 1. Upload chain packages
for pkg in chain_base chain_mid chain_top; do
    log_message "$LOGGING_LEVEL" "info" "Uploading $pkg.tar.gz..."
    response=$(curl -s -X POST -F "package=@${PKGS_DIR}/${pkg}.tar.gz" "${SERVER_URL}/packages/upload/")
    if echo "$response" | grep -q '"result":"success"'; then
        log_message "$LOGGING_LEVEL" "info" "Uploaded $pkg"
    else
        log_message "$LOGGING_LEVEL" "error" "Failed to upload $pkg: $response"
        FAILURES=$((FAILURES + 1))
    fi
done

# 2. Install base → mid → top
for pkg in chain_base.tar.gz chain_mid.tar.gz chain_top.tar.gz; do
    log_message "$LOGGING_LEVEL" "info" "Installing $pkg..."
    response=$(curl -s -X POST -H "Content-Type: application/json" \
        -d "{\"filename\": \"$pkg\"}" "${SERVER_URL}/packages/install/")
    if echo "$response" | grep -q '"result":"success"'; then
        log_message "$LOGGING_LEVEL" "info" "Installed $pkg"
    else
        log_message "$LOGGING_LEVEL" "error" "Failed to install $pkg: $response"
        FAILURES=$((FAILURES + 1))
    fi
done

if [ $FAILURES -gt 0 ]; then
    log_message "$LOGGING_LEVEL" "error" "Setup failed"
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$WORKSPACE_DIR" || true
    exit 1
fi

# 3. Recursive uninstall: top first, then mid, then base
for pkg in chain_top chain_mid chain_base; do
    log_message "$LOGGING_LEVEL" "info" "Uninstalling $pkg..."
    response=$(curl -s -X POST -H "Content-Type: application/json" \
        -d "{\"package\": \"$pkg\"}" "${SERVER_URL}/packages/uninstall")
    if echo "$response" | grep -q '"result":"success"'; then
        log_message "$LOGGING_LEVEL" "info" "Uninstalled $pkg"
    else
        log_message "$LOGGING_LEVEL" "error" "Failed to uninstall $pkg: $response"
        FAILURES=$((FAILURES + 1))
    fi
done

# Verify all removed
for pkg in chain_base chain_mid chain_top; do
    if is_installed "$pkg"; then
        log_message "$LOGGING_LEVEL" "error" "$pkg still installed!"
        FAILURES=$((FAILURES + 1))
    fi
done

"$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$WORKSPACE_DIR" || true

if [ $FAILURES -eq 0 ]; then
    rm -rf "$TEST_DIR/conf" "$TEST_DIR/logs" "$TEST_PID_FILE" "$STAGING_DIR"
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit 0
else
    log_message "$LOGGING_LEVEL" "error" "Test FAILED with $FAILURES failures"
    exit 1
fi
