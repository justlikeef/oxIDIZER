#!/bin/bash
# 000001-ConfigInvalid/test.sh

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}
BROKER_PORT=${PORTS[1]}

source "$TEST_LIBS_DIR/log_function.sh"
log_message "$LOGGING_LEVEL" "info" "Starting Test: 000001-ConfigInvalid"

# Define workspace and paths
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"
SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"

# 1. Create BROKEN broker config
mkdir -p "$SCRIPT_DIR/conf"
BAD_BROKER_CONFIG="$SCRIPT_DIR/conf/bad_broker.yaml"
cat <<EOF > "$BAD_BROKER_CONFIG"
invalid:
  - yaml
    - structure
EOF

# 2. Start Server with main config pointing to bad broker config
MAIN_CONFIG="$SCRIPT_DIR/conf/ox_webservice.runtime.yaml"
cat <<EOF > "$MAIN_CONFIG"
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: messaging_mqtt
    name: ox_messaging_mqtt
    path: "$WORKSPACE_DIR/target/$TARGET/libox_messaging_mqtt.so"
    broker_port: $BROKER_PORT
    config_file: "$BAD_BROKER_CONFIG"

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

LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"
mkdir -p "$SCRIPT_DIR/logs"

log_message "$LOGGING_LEVEL" "info" "Starting server with bad broker config..."
"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$MAIN_CONFIG" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1

sleep 5

# Read actual Server PID
if [ -f "$PID_FILE" ]; then
    SERVER_PID=$(cat "$PID_FILE")
    log_message "$LOGGING_LEVEL" "info" "Server PID from file: $SERVER_PID"
else
    log_message "$LOGGING_LEVEL" "error" "PID file not found!"
    if [ -f "$SCRIPT_DIR/start_script.log" ]; then cat "$SCRIPT_DIR/start_script.log"; fi
    if [ -f "$LOG_FILE" ]; then cat "$LOG_FILE"; fi
    exit 1
fi

# 5. Verify Fallback/Survival
# If it survived, it's a pass for this specific test (robustness)
if kill -0 "$SERVER_PID" 2>/dev/null; then
    log_message "$LOGGING_LEVEL" "info" "Server survived bad module config (Success)."
    STATUS=0
else
    log_message "$LOGGING_LEVEL" "error" "Server crashed causing PID $SERVER_PID to die."
    cat "$LOG_FILE"
    STATUS=1
fi

# 6. Cleanup
kill "$SERVER_PID" 2>/dev/null
# rm -rf "$SCRIPT_DIR/conf" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log"

exit $STATUS
