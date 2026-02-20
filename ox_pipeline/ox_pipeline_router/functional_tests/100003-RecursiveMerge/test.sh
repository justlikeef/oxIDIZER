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
mkdir -p "$TEST_DIR/logs"

# Test Setup for Recursive Merge
TEST_DIR=$(dirname "$(readlink -f "$0")")
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
mkdir -p "$TEST_DIR/logs"

# Config Setup
# Create nested structure
mkdir -p "$TEST_DIR/conf/nested/level1/level2"

# Dynamic Config using Heredoc
cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"

merge_recursive:
  - "$TEST_DIR/conf/nested/"

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
EOF

# Create DEEP config file
DEEP_CONFIG_FILE="$TEST_DIR/conf/nested/level1/level2/deep_module.yaml"
cat <<EOF > "$DEEP_CONFIG_FILE"
modules:
  - id: 'deep_module'
    name: 'ox_pipeline_router'
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_pipeline_router.so"

routes:
  - url: "^/deep"
    module_id: "deep_module"
    phase: Content
    priority: 100
EOF

log_message "$LOGGING_LEVEL" "info" "Starting server with recursive merge config..."

"$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$LOG_FILE" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

sleep 5

# Check if running
if [ -f "$TEST_PID_FILE" ] && kill -0 $(cat "$TEST_PID_FILE") 2>/dev/null; then
    log_message "$LOGGING_LEVEL" "info" "Server is running (PASS)"
else
    log_message "$LOGGING_LEVEL" "error" "Server crashed (FAIL)"
    if [ -f "$LOG_FILE" ]; then cat "$LOG_FILE"; fi
    exit 1
fi

# Verify module loaded by hitting the route
RESP=$(curl --connect-timeout 30 --max-time 60 -s "http://127.0.0.1:$BASE_PORT/deep")
# The router returns 404 if no match, or 200/404 from module. 
# ox_pipeline_router itself doesn't return content, but simply existing means it routed?
# Wait, ox_pipeline_router is a router. If we route to it, it routes again?
# Actually, if we use `ox_pipeline_router` as the target module, it might just return 404 or nothing if no sub-routes.
# But checking if it returned a valid HTTP response (even 404) from the server means it's running.
# Better: Check logs for "Loading module 'deep_module'"

if grep -q "Loading module 'deep_module'" "$LOG_FILE"; then
    log_message "$LOGGING_LEVEL" "info" "Deep module loaded recursively (PASS)"
else
    log_message "$LOGGING_LEVEL" "error" "Deep module NOT loaded (FAIL)"
    echo "=== LOG OUTPUT ==="
    cat "$LOG_FILE"
    echo "=================="
    FAILURES=1
fi

"$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
rm -rf "$TEST_DIR/conf" "$TEST_DIR/logs" "$TEST_PID_FILE" 

if [ "$FAILURES" == "1" ]; then
    exit 1
else
    exit 0
fi
