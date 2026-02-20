#!/bin/bash
set -e

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR=${6:-"3005 3006 3007 3008 3009"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

source "$TEST_LIBS_DIR/log_function.sh"

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
SERVER_STOP_SCRIPT="$WORKSPACE_DIR/scripts/stop_server.sh"

log_message "$LOGGING_LEVEL" "info" "Starting Test: 200002-ConflictManagement"

# Replace placeholder in config
sed "s/%BASE_PORT%/$BASE_PORT/g" "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml" > "$SCRIPT_DIR/conf/ox_webservice.active.yaml"

# Reset data sources
mkdir -p "$SCRIPT_DIR/conf/datastores"
rm -f "$SCRIPT_DIR/conf/datastores"/*

LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$SCRIPT_DIR/conf/ox_webservice.active.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &

sleep 3

log_message "$LOGGING_LEVEL" "info" "1. Testing Conflict Skip (/ping)"
# Ping module handles /ping (priority 100), then DataStoreManager (priority 200) skips.
RESP=$(curl -s "http://localhost:$BASE_PORT/ping")
if ! echo "$RESP" | grep -q "pong"; then
    log_message "$LOGGING_LEVEL" "error" "Conflict skip failed! Expected pong, got: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "Conflict skip OK"

log_message "$LOGGING_LEVEL" "info" "2. Testing Conflict Error (/conflict_error)"
# Ping handles priority 100, then DataStoreManager (priority 200) throws error.
STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:$BASE_PORT/conflict_error")
if [ "$STATUS" != "500" ]; then
    log_message "$LOGGING_LEVEL" "error" "Conflict error failed! Expected 500, got: $STATUS"
    exit 1
fi
RESP=$(curl -s "http://localhost:$BASE_PORT/conflict_error")
if ! echo "$RESP" | grep -q "Conflict"; then
    log_message "$LOGGING_LEVEL" "error" "Conflict error message missing! Got: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "Conflict error OK"

# Cleanup
"$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
rm -rf "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log" "$SCRIPT_DIR/conf/ox_webservice.active.yaml"
exit 0
