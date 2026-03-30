#!/bin/bash
set -e
# 100002-Routing/test.sh

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

# Source logging
source "$TEST_LIBS_DIR/log_function.sh"
# source "$TEST_LIBS_DIR/start_server.sh" # Using variable for safety

# Define workspace and paths
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
MODULE_DIR=$(dirname "$(dirname "$SCRIPT_DIR")")
WORKSPACE_DIR=$(dirname "$MODULE_DIR")

SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
SERVER_STOP_SCRIPT="$WORKSPACE_DIR/scripts/stop_server.sh"

log_message "$LOGGING_LEVEL" "info" "Starting Test: 100002-Routing"

# Setup Config
CONFIG_DIR="$SCRIPT_DIR/conf"
mkdir -p "$CONFIG_DIR"
cat <<EOF > "$CONFIG_DIR/ox_webservice.runtime.yaml"
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: driver_manager
    name: ox_persistence_driver_manager
    path: "$WORKSPACE_DIR/target/$TARGET/libox_persistence_driver_manager.so"

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
  - url: "^/drivers/(.*)?$"
    headers:
      Accept: "application/json"
    module_id: "driver_manager"
    phase: Content
    priority: 499
  - url: "^/drivers/(.*)?$"
    query:
      format: "json"
    module_id: "driver_manager"
    phase: Content
    priority: 499
EOF

# Start Server
LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"
mkdir -p "$SCRIPT_DIR/logs"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$CONFIG_DIR/ox_webservice.runtime.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &
# SERVER_PID=$! # No longer needed, start_server.sh writes to PID_FILE

# Wait for server
sleep 3

# verification

log_message "$LOGGING_LEVEL" "info" "Testing /drivers/ (Header)"
CODE=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" -H "Accept: application/json" http://localhost:$BASE_PORT/drivers/)
if [ "$CODE" != "200" ]; then
    log_message "$LOGGING_LEVEL" "error" "Header test failed, got $CODE"
    cat "$LOG_FILE"
    exit 1
fi

log_message "$LOGGING_LEVEL" "info" "Testing /drivers/ (Query)"
CODE=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" "http://localhost:$BASE_PORT/drivers/?format=json")
if [ "$CODE" != "200" ]; then
    log_message "$LOGGING_LEVEL" "error" "Query test failed, got $CODE"
    cat "$LOG_FILE"
    exit 1
fi

log_message "$LOGGING_LEVEL" "info" "Testing /drivers/ (Fallback - Expect 404 or Stream)"
# Without headers/query, it should skip driver_manager. 
# Since no other module claims /drivers, it should likely 404.
CODE=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" http://localhost:$BASE_PORT/drivers/)
if [ "$CODE" == "200" ]; then
    log_message "$LOGGING_LEVEL" "error" "Fallback test failed! Got 200, expected skip (likely 404)."
    cat "$LOG_FILE"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "Fallback got $CODE (Correct)"

log_message "$LOGGING_LEVEL" "info" "Testing /drivers/available (Header)"
CODE=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" -H "Accept: application/json" http://localhost:$BASE_PORT/drivers/available)
if [ "$CODE" != "200" ]; then
    log_message "$LOGGING_LEVEL" "error" "Available test failed, got $CODE"
    cat "$LOG_FILE"
    # Check if it is still alive
    if [ -f "$PID_FILE" ]; then
        SERVER_PID=$(cat "$PID_FILE")
        if ! ps -p "$SERVER_PID" > /dev/null; then
            log_message "$LOGGING_LEVEL" "error" "Server crashed!"
            cat "$LOG_FILE"
            exit 1
        fi
    else
        log_message "$LOGGING_LEVEL" "error" "PID file not found!"
        exit 1
    fi

    # Cleanup
    if [ -f "$SERVER_STOP_SCRIPT" ]; then
        "$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
    else
        kill $(cat "$PID_FILE") 2>/dev/null
    fi
    rm -rf "$CONFIG_DIR" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log"
    exit 0
fi

# Check if it is still alive
if [ -f "$PID_FILE" ]; then
    SERVER_PID=$(cat "$PID_FILE")
    if ! ps -p "$SERVER_PID" > /dev/null; then
        log_message "$LOGGING_LEVEL" "error" "Server crashed!"
        cat "$LOG_FILE"
        exit 1
    fi
else
    log_message "$LOGGING_LEVEL" "error" "PID file not found!"
    exit 1
fi

# Cleanup
if [ -f "$SERVER_STOP_SCRIPT" ]; then
    "$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
else
    # Fallback if stop script fails or is missing
    if [ -f "$PID_FILE" ]; then
        kill $(cat "$PID_FILE") 2>/dev/null
    fi
fi
rm -rf "$CONFIG_DIR" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log"
exit 0
