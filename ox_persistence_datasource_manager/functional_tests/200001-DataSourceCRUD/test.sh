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

log_message "$LOGGING_LEVEL" "info" "Starting Test: 200001-DataSourceCRUD"

# Replace placeholder in config
sed "s/%BASE_PORT%/$BASE_PORT/g" "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml" > "$SCRIPT_DIR/conf/ox_webservice.active.yaml"

# 0. Setup initial data
mkdir -p "$SCRIPT_DIR/conf/datastores"
cat > "$SCRIPT_DIR/conf/datastores/ds1.yaml" <<EOF
id: "ds1"
name: "Primary PostgreSQL"
driver_id: "postgres"
config:
  host: "localhost"
  port: 5432
EOF

LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$SCRIPT_DIR/conf/ox_webservice.active.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &

sleep 3

log_message "$LOGGING_LEVEL" "info" "1. Testing GET /data_sources"
RESP=$(curl -s -H "Accept: application/json" "http://localhost:$BASE_PORT/data_sources")
if ! echo "$RESP" | grep -q "ds1" || ! echo "$RESP" | grep -q "Primary PostgreSQL"; then
    log_message "$LOGGING_LEVEL" "error" "GET /data_sources failed! Response: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "GET /data_sources OK"

log_message "$LOGGING_LEVEL" "info" "2. Testing POST /data_sources"
NEW_DS='{"id":"ds2","name":"Secondary MySQL","driver_id":"mysql","config":{"host":"remote-host","port":3306}}'
RESP=$(curl -s -X POST -H "Content-Type: application/json" -H "Accept: application/json" -d "$NEW_DS" "http://localhost:$BASE_PORT/data_sources")
if ! echo "$RESP" | grep -q "created"; then
    log_message "$LOGGING_LEVEL" "error" "POST /data_sources failed! Response: $RESP"
    exit 1
fi
# Verify it was added
RESP=$(curl -s -H "Accept: application/json" "http://localhost:$BASE_PORT/data_sources")
if ! echo "$RESP" | grep -q "ds2"; then
    log_message "$LOGGING_LEVEL" "error" "POST /data_sources verify failed! Response: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "POST /data_sources OK"

log_message "$LOGGING_LEVEL" "info" "3. Testing DELETE /data_sources/ds1"
RESP=$(curl -s -X DELETE -H "Accept: application/json" "http://localhost:$BASE_PORT/data_sources/ds1")
if ! echo "$RESP" | grep -q "deleted"; then
    log_message "$LOGGING_LEVEL" "error" "DELETE /data_sources/ds1 failed! Response: $RESP"
    exit 1
fi
# Verify it was deleted
RESP=$(curl -s -H "Accept: application/json" "http://localhost:$BASE_PORT/data_sources")
if echo "$RESP" | grep -q "ds1"; then
    log_message "$LOGGING_LEVEL" "error" "DELETE /data_sources/ds1 verify failed! Response: $RESP"
    exit 1
fi
log_message "$LOGGING_LEVEL" "info" "DELETE /data_sources OK"

# Cleanup
"$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
rm -rf "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log" "$SCRIPT_DIR/conf/ox_webservice.active.yaml"
exit 0
