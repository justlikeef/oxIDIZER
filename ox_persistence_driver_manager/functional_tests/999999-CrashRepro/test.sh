#!/bin/bash
set -e
# 999999-CrashRepro/test.sh

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

# Define workspace and paths
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
MODULE_DIR=$(dirname "$(dirname "$SCRIPT_DIR")")
WORKSPACE_DIR=$(dirname "$MODULE_DIR")

SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
SERVER_STOP_SCRIPT="$WORKSPACE_DIR/scripts/stop_server.sh"

log_message "$LOGGING_LEVEL" "info" "Starting Test: 999999-CrashRepro"

# Define workspace and paths
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
MODULE_DIR=$(dirname "$(dirname "$SCRIPT_DIR")")
WORKSPACE_DIR=$(dirname "$MODULE_DIR")

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
EOF

# Start Server
LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"
mkdir -p "$SCRIPT_DIR/logs"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$CONFIG_DIR/ox_webservice.runtime.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &

# Wait for server
sleep 2

# Verification
# This should NOT crash the server
log_message "$LOGGING_LEVEL" "info" "Making request to http://localhost:$BASE_PORT/drivers"
curl --connect-timeout 30 --max-time 60 -v "http://localhost:$BASE_PORT/drivers" > "$SCRIPT_DIR/curl_output.txt" 2>&1 || true

# Check if it is still alive
if ! ps -p $(cat "$PID_FILE") > /dev/null; then
    log_message "$LOGGING_LEVEL" "error" "Server crashed!"
    cat "$LOG_FILE"
    exit 1
fi

log_message "$LOGGING_LEVEL" "info" "Server survived."

# Cleanup
if [ -f "$SERVER_STOP_SCRIPT" ]; then
    "$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
else
    if [ -f "$PID_FILE" ]; then
        kill $(cat "$PID_FILE") 2>/dev/null
    fi
fi
rm -rf "$CONFIG_DIR" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log" "$SCRIPT_DIR/curl_output.txt"
exit 0
