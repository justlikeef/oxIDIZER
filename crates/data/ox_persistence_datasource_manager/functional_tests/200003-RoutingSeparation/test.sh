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

log_message "$LOGGING_LEVEL" "info" "Starting Test: 200003-RoutingSeparation"

# Replace placeholder in config
sed "s/%BASE_PORT%/$BASE_PORT/g" "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml" > "$SCRIPT_DIR/conf/ox_webservice.active.yaml"

# Setup initial data
mkdir -p "$SCRIPT_DIR/conf/datastores"
rm -f "$SCRIPT_DIR/conf/datastores"/*

LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$SCRIPT_DIR/conf/ox_webservice.active.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &

sleep 3

log_message "$LOGGING_LEVEL" "info" "1. Testing Accept: application/json (Should reach API)"
RESP=$(curl -s -H "Accept: application/json" "http://localhost:$BASE_PORT/data_sources")
if ! echo "$RESP" | grep -q "data_sources"; then
    log_message "$LOGGING_LEVEL" "error" "API request failed! Expected JSON, got: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "API Accept header OK"

log_message "$LOGGING_LEVEL" "info" "2. Testing format=json (Should reach API)"
RESP=$(curl -s "http://localhost:$BASE_PORT/data_sources?format=json")
if ! echo "$RESP" | grep -q "data_sources"; then
    log_message "$LOGGING_LEVEL" "error" "API format=json failed! Expected JSON, got: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "API format=json OK"

log_message "$LOGGING_LEVEL" "info" "3. Testing Content (Should reach Stream, index.html)"
RESP=$(curl -s "http://localhost:$BASE_PORT/data_sources/")
if ! echo "$RESP" | grep -q "<title>Data Source Manager | oxIDIZER</title>"; then
    log_message "$LOGGING_LEVEL" "error" "Content request failed! Expected HTML, got: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "Content Stream OK"

log_message "$LOGGING_LEVEL" "info" "4. Testing Asset (Should reach Stream, css/style.css)"
RESP=$(curl -s "http://localhost:$BASE_PORT/data_sources/css/style.css")
if ! echo "$RESP" | grep -q "background-color"; then
    log_message "$LOGGING_LEVEL" "error" "Asset request failed! Expected CSS, got: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "Asset Stream OK"


# Cleanup
"$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
# rm -rf "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log" "$SCRIPT_DIR/conf/ox_webservice.active.yaml"
exit 0
