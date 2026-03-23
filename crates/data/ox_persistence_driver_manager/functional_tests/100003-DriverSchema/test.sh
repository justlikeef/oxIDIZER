#!/bin/bash
set -e

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

source "$TEST_LIBS_DIR/log_function.sh"

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
SERVER_STOP_SCRIPT="$WORKSPACE_DIR/scripts/stop_server.sh"

log_message "$LOGGING_LEVEL" "info" "Starting Test: 100003-DriverSchema"

# Replace placeholder in config
sed "s/%BASE_PORT%/$BASE_PORT/g" "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml" > "$SCRIPT_DIR/conf/ox_webservice.active.yaml"

LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$SCRIPT_DIR/conf/ox_webservice.active.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &

sleep 3

log_message "$LOGGING_LEVEL" "info" "1. Testing /drivers/?state=enabled"
RESP=$(curl -s "http://localhost:$BASE_PORT/drivers/?state=enabled")
if ! echo "$RESP" | grep -q "postgres" || ! echo "$RESP" | grep -q "api"; then
    log_message "$LOGGING_LEVEL" "error" "Filtering failed! Response: $RESP"
    # Try with verbose if it failed
    curl -v "http://localhost:$BASE_PORT/drivers/?state=enabled"
    exit 1
fi
if echo "$RESP" | grep -q "json"; then
    log_message "$LOGGING_LEVEL" "error" "Filtering failed! Found 'json' driver which should be disabled. Response: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "Filtering OK"

log_message "$LOGGING_LEVEL" "info" "2. Testing /drivers/postgres/schema"
RESP=$(curl -s "http://localhost:$BASE_PORT/drivers/postgres/schema")
if ! echo "$RESP" | grep -q "postgres_datasource_form"; then
    log_message "$LOGGING_LEVEL" "error" "Postgres schema retrieval failed! Response: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "Postgres schema OK"

log_message "$LOGGING_LEVEL" "info" "3. Testing /drivers/api/schema"
RESP=$(curl -s "http://localhost:$BASE_PORT/drivers/api/schema")
if ! echo "$RESP" | grep -q "api_datasource_form"; then
    log_message "$LOGGING_LEVEL" "error" "API schema retrieval failed! Response: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "API schema OK"

# Cleanup
"$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
rm -rf "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log" "$SCRIPT_DIR/conf/ox_webservice.active.yaml"
exit 0
