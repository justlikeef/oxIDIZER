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
source "$TEST_LIBS_DIR/log_function.sh"
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
mkdir -p "$TEST_DIR/conf"
cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: package_manager
    name: ox_package_manager
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_package_manager.so"
    staging_directory: "$TEST_DIR/staging"
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
  - url: "^/packages/(upload|list|install)/?"
    module_id: "package_manager"
    phase: Content
    priority: 450
EOF

# Ensure clean state
rm -rf "$TEST_DIR/pkg_content" "$TEST_DIR/installed"
mkdir -p "$TEST_DIR/staging"

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
FILENAME="install_test_pkg.zip"
SAMPLE_ZIP="$TEST_WORKSPACE_DIR/sample_projects/sample_project_yaml.zip"
if [ ! -f "$SAMPLE_ZIP" ]; then
    log_message "$LOGGING_LEVEL" "error" "Sample zip not found at $SAMPLE_ZIP"
    ls -R "$TEST_WORKSPACE_DIR/sample_projects"
    exit 1
fi
cp "$SAMPLE_ZIP" "$TEST_DIR/$FILENAME"

# 1. Upload Package
log_message "$LOGGING_LEVEL" "info" "Uploading $FILENAME to http://127.0.0.1:$BASE_PORT/packages/upload/"
curl --connect-timeout 30 --max-time 60 -s -X POST -F "package=@$TEST_DIR/$FILENAME" "http://127.0.0.1:$BASE_PORT/packages/upload/" > /dev/null

# 2. Verify List
log_message "$LOGGING_LEVEL" "info" "Verifying list..."
LIST_RESPONSE=$(curl --connect-timeout 30 --max-time 60 -s "http://127.0.0.1:$BASE_PORT/packages/list/")
if echo "$LIST_RESPONSE" | grep -q "$FILENAME"; then
    log_message "$LOGGING_LEVEL" "info" "Package uploaded and listed."
else
    log_message "$LOGGING_LEVEL" "error" "Package failed to list. Response: $LIST_RESPONSE"
    FAILURES=$((FAILURES + 1))
fi

# 3. Install Package
log_message "$LOGGING_LEVEL" "info" "Sending Install Request..."
INSTALL_RESPONSE=$(curl --connect-timeout 30 --max-time 60 -s -X POST -H "Content-Type: application/json" -d "{\"filename\": \"$FILENAME\"}" "http://127.0.0.1:$BASE_PORT/packages/install/")
log_message "$LOGGING_LEVEL" "debug" "Install Response: $INSTALL_RESPONSE"

if echo "$INSTALL_RESPONSE" | grep -q "success"; then
    log_message "$LOGGING_LEVEL" "info" "Install API returned success."
else
    log_message "$LOGGING_LEVEL" "error" "Install API failed. Response: $INSTALL_RESPONSE"
    if [ -f "$LOG_FILE" ]; then
        log_message "$LOGGING_LEVEL" "debug" "Server Log Dump:"
        cat "$LOG_FILE"
    fi
    FAILURES=$((FAILURES + 1))
fi

# 4. Verify Removed from List
LIST_AFTER=$(curl --connect-timeout 30 --max-time 60 -s "http://127.0.0.1:$BASE_PORT/packages/list/")
if echo "$LIST_AFTER" | grep -q "$FILENAME"; then
     log_message "$LOGGING_LEVEL" "error" "Package still in list after install!"
     if [ -f "$LOG_FILE" ]; then
        log_message "$LOGGING_LEVEL" "debug" "Server Log Dump (Cleanup check):"
        cat "$LOG_FILE"
     fi
     FAILURES=$((FAILURES + 1))
else
     log_message "$LOGGING_LEVEL" "info" "Package removed from staged list."
fi

# Cleanup
"$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
rm -rf "$TEST_DIR/conf" "$TEST_DIR/logs" "$TEST_PID_FILE" "$TEST_DIR/$FILENAME"

if [ $FAILURES -eq 0 ]; then
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit 0
else
    log_message "$LOGGING_LEVEL" "error" "Test FAILED with $FAILURES failures"
    exit 1
fi
