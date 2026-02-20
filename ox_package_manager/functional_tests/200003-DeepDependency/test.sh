#!/bin/bash

# Arguments
SCRIPTS_DIR=$1
TEST_LIBS_DIR=$2
MODE=$3
LOGGING_LEVEL=$4
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

# Setup
source "/var/repos/oxIDIZER/functional_tests/common/log_function.sh"
TEST_DIR=$(dirname "$(readlink -f "$0")")
TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"

if [ "$MODE" == "integrated" ]; then
    log_message "$LOGGING_LEVEL" "info" "Skipping in integrated mode"
    exit 77
fi

# Setup isolated environment
TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
LOG_FILE="$TEST_DIR/logs/ox_webservice.log"
mkdir -p "$TEST_DIR/logs" "$TEST_DIR/conf"

# Config Setup
cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: package_manager
    name: ox_package_manager
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_package_manager.so"
    staging_directory: "$TEST_DIR/staging"
    manifests_directory: "$TEST_DIR/installed"
  - id: ox_pipeline_router
    name: ox_pipeline_router
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_pipeline_router.so"

servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "0.0.0.0"
    hosts:
      - name: "localhost"

pipeline:
  phases:
    - Content: "ox_pipeline_router"

routes:
  - url: "^/packages/upload/?"
    module_id: "package_manager"
    phase: Content
    priority: 450
  - url: "^/packages/(list|install|uninstall)(/.*)?$"
    module_id: "package_manager"
    phase: Content
    priority: 450
EOF

# Ensure clean state
rm -rf "$TEST_DIR/staging" "$TEST_DIR/installed"
mkdir -p "$TEST_DIR/staging" "$TEST_DIR/installed"

# Start Server
"$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$LOG_FILE" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

sleep 5

if [ ! -f "$TEST_PID_FILE" ] || ! kill -0 $(cat "$TEST_PID_FILE") 2>/dev/null; then
    log_message "$LOGGING_LEVEL" "error" "Server failed to start"
    if [ -f "$LOG_FILE" ]; then cat "$LOG_FILE"; fi
    exit 1
fi

FAILURES=0

# Prepare Test Data
PKG_A_ZIP="$TEST_WORKSPACE_DIR/ox_package_manager/test_pkgs/deep_dep_a.tar.gz"
PKG_B_ZIP="$TEST_WORKSPACE_DIR/ox_package_manager/test_pkgs/deep_dep_b.tar.gz"
PKG_C_ZIP="$TEST_WORKSPACE_DIR/ox_package_manager/test_pkgs/deep_dep_c.tar.gz"

cp "$PKG_A_ZIP" "$TEST_DIR/deep_dep_a.tar.gz"
cp "$PKG_B_ZIP" "$TEST_DIR/deep_dep_b.tar.gz"
cp "$PKG_C_ZIP" "$TEST_DIR/deep_dep_c.tar.gz"

# 1. Upload Packages
for pkg in deep_dep_a deep_dep_b deep_dep_c; do
    log_message "$LOGGING_LEVEL" "info" "Uploading $pkg..."
    UPLOAD_RES=$(curl -s -X POST -F "package=@$TEST_DIR/$pkg.tar.gz" "http://127.0.0.1:$BASE_PORT/packages/upload/")
    log_message "$LOGGING_LEVEL" "info" "Upload result: $UPLOAD_RES"
done

# 2. Install A
log_message "$LOGGING_LEVEL" "info" "Installing deep_dep_a..."
INSTALL_RESPONSE=$(curl -s -X POST -H "Content-Type: application/json" -d "{\"filename\": \"deep_dep_a.tar.gz\"}" "http://127.0.0.1:$BASE_PORT/packages/install/")
log_message "$LOGGING_LEVEL" "debug" "Response: $INSTALL_RESPONSE"

if echo "$INSTALL_RESPONSE" | grep -q "success"; then
     log_message "$LOGGING_LEVEL" "info" "Install returned success."
else
     log_message "$LOGGING_LEVEL" "error" "Install failed."
     FAILURES=$((FAILURES + 1))
fi

# 3. Verify all installed
log_message "$LOGGING_LEVEL" "info" "Verifying installation of all dependencies..."
LIST_RESPONSE=$(curl -s "http://127.0.0.1:$BASE_PORT/packages/list/installed")

log_message "$LOGGING_LEVEL" "debug" "Installed List: $LIST_RESPONSE"

if echo "$LIST_RESPONSE" | grep -q "deep_dep_a" && \
   echo "$LIST_RESPONSE" | grep -q "deep_dep_b" && \
   echo "$LIST_RESPONSE" | grep -q "deep_dep_c"; then
     log_message "$LOGGING_LEVEL" "info" "All packages verified installed."
else
     log_message "$LOGGING_LEVEL" "error" "Missing packages in installed list."
     FAILURES=$((FAILURES + 1))
fi

# Cleanup
"$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

if [ $FAILURES -eq 0 ]; then
    rm -rf "$TEST_DIR/conf" "$TEST_DIR/logs" "$TEST_PID_FILE" "$TEST_DIR/staging" "$TEST_DIR/installed" "$TEST_DIR/deep_dep_*.tar.gz"
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit 0
else
    # Preserve logs for debugging
    log_message "$LOGGING_LEVEL" "error" "Test FAILED with $FAILURES failures"
    log_message "$LOGGING_LEVEL" "info" "Logs preserved in $TEST_DIR/logs"
    exit 1
fi
