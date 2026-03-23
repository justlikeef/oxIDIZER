#!/bin/bash
# 100001-Verification/test.sh

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR="${6:-"3000 3001 3002 3003 3004"}"
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}
BROKER_PORT=${PORTS[1]}
CONSOLE_PORT=${PORTS[2]}

source "$TEST_LIBS_DIR/log_function.sh"
log_message "$LOGGING_LEVEL" "info" "Starting Test: 100001-Verification (Ports: $PORTS_STR)"

# Define workspace and paths
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
MODULE_DIR=$(dirname "$(dirname "$SCRIPT_DIR")")
WORKSPACE_DIR=$(dirname "$MODULE_DIR")
SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"

# Ensure ports are clear
log_message "$LOGGING_LEVEL" "info" "Cleaning up ports $PORTS_STR..."
for p in "${PORTS[@]}"; do
    fuser -k "$p/tcp" 2>/dev/null || true
done
sleep 1

# Runtime Config
mkdir -p "$SCRIPT_DIR/conf"

# Generate Broker Config
cat <<EOF > "$SCRIPT_DIR/conf/broker.runtime.yaml"
id: 0
console:
  listen: "0.0.0.0:$CONSOLE_PORT"
router:
  max_segment_size: 1048576
  max_segment_count: 10
  max_connections: 10
  max_outgoing_packet_count: 200
v4:
  v4.1:
    name: "v4-1"
    listen: "0.0.0.0:$BROKER_PORT"
    next_connection_delay_ms: 1
    connections:
      max_payload_size: 2048
      max_inflight_count: 100
      connection_timeout_ms: 100
EOF

# Generate WebService Config
cat <<EOF > "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: messaging_mqtt
    name: ox_messaging_mqtt
    path: "$WORKSPACE_DIR/target/$TARGET/libox_messaging_mqtt.so"
    config_file: "$SCRIPT_DIR/conf/broker.runtime.yaml"

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

CONFIG_FILE="$SCRIPT_DIR/conf/ox_webservice.runtime.yaml"
LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"
mkdir -p "$SCRIPT_DIR/logs"

# Ensure server is running for this test
# Check if running
if ! ss -tuln | grep ":$BROKER_PORT " > /dev/null; then
    log_message "$LOGGING_LEVEL" "info" "Broker not running on $BROKER_PORT. Starting..."
    "$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$CONFIG_FILE" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &
    # We wait for the PID file to be created by start_server.sh
    sleep 5
else
    log_message "$LOGGING_LEVEL" "info" "Broker already running on $BROKER_PORT."
fi

# Run Verify Messaging Binary
log_message "$LOGGING_LEVEL" "info" "Running verify_messaging binary with MQTT_PORT=$BROKER_PORT..."
cd "$WORKSPACE_DIR"
export MQTT_PORT=$BROKER_PORT
if cargo run -p ox_messaging_client --bin verify_messaging > "$SCRIPT_DIR/client_output.log" 2>&1; then
    log_message "$LOGGING_LEVEL" "info" "verify_messaging PASSED."
    STATUS=0
else
    log_message "$LOGGING_LEVEL" "error" "verify_messaging FAILED."
    cat "$SCRIPT_DIR/client_output.log"
    STATUS=1
    
    # Debug server log if failed
    if [ -f "$SCRIPT_DIR/server_output.log" ]; then
         cat "$SCRIPT_DIR/server_output.log"
    fi
fi

if [ -f "$PID_FILE" ]; then
    kill $(cat "$PID_FILE") 2>/dev/null
fi

if [ "$STATUS" -eq 0 ]; then
    rm -rf "$SCRIPT_DIR/conf" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/client_output.log" "$SCRIPT_DIR/start_script.log"
fi

exit $STATUS
