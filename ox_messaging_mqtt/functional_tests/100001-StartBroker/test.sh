#!/bin/bash
# 100001-StartBroker/test.sh

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}
BROKER_PORT=${PORTS[1]}
CONSOLE_PORT=${PORTS[2]}

# Source logging
source "$TEST_LIBS_DIR/log_function.sh"

log_message "$LOGGING_LEVEL" "info" "Starting Test: 100001-StartBroker"

# Define workspace and paths
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"

# Ensure clean state
sleep 1

# Start Server
   # Runtime Config
   mkdir -p "$SCRIPT_DIR/conf"
   cat <<EOF > "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: messaging_mqtt
    name: ox_messaging_mqtt
    path: "$WORKSPACE_DIR/target/$TARGET/libox_messaging_mqtt.so"
    broker_port: $BROKER_PORT
    console_port: $CONSOLE_PORT

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

   # Use runtime config in start script argument if possible (start_server.sh takes config as arg 3)
   CONFIG_FILE="$SCRIPT_DIR/conf/ox_webservice.runtime.yaml"
   LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
   PID_FILE="$SCRIPT_DIR/ox_webservice.pid"
   mkdir -p "$SCRIPT_DIR/logs"

log_message "$LOGGING_LEVEL" "info" "Starting server..."
"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$CONFIG_FILE" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1
# script exits, sleep to allow server start
# Polling for port availability (max 15 seconds)
MAX_RETRIES=15
for ((i=1; i<=MAX_RETRIES; i++)); do
    if ss -tuln | awk '{print $5}' | grep -q ":$BROKER_PORT$"; then
        log_message "$LOGGING_LEVEL" "info" "Port $BROKER_PORT is OPEN. Verification Successful."
        
        # Clean up
        kill "$SERVER_PID" 2>/dev/null
        rm -rf "$SCRIPT_DIR/conf" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log"
        exit 0
    fi
    sleep 1
done

log_message "$LOGGING_LEVEL" "error" "Port $BROKER_PORT is NOT OPEN after $MAX_RETRIES seconds."
cat "$LOG_FILE"

# Clean up
kill "$SERVER_PID" 2>/dev/null
rm -rf "$SCRIPT_DIR/conf" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log"
exit 1
