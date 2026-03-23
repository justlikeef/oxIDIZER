#!/bin/bash
# 500001-ConnectionLimit/test.sh

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
log_message "$LOGGING_LEVEL" "info" "Starting Test: 500001-ConnectionLimit"

# Define paths
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"
SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
# 2. Configure Max Connections = 1 (and console port)
CONSOLE_PORT=${PORTS[2]}
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
    max_connections: 2

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

log_message "$LOGGING_LEVEL" "info" "Starting server with max_connections=1..."
"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$CONFIG_FILE" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1
sleep 5

# Check if running
if [ ! -f "$PID_FILE" ]; then
     log_message "$LOGGING_LEVEL" "error" "Server failed to start."
     cat "$LOG_FILE"
     exit 1
fi
SERVER_PID=$(cat "$PID_FILE")

# 5. Connect Client 1 (Consume connection)
# Use nc to hold connection open. 
# Send MQTT CONNECT packet? 
# Start Client 1
# Minimal MQTT 3.1.1 Verify: 10 13 00 04 4D 51 54 54 04 02 00 3C 00 07 console
# 5. Connect Client 1 (Consume connection)
log_message "$LOGGING_LEVEL" "info" "Starting Client 1..."
python3 -u "$WORKSPACE_DIR/scripts/check_connection.py" "0.0.0.0" "$BROKER_PORT" "c1" 60 > "$SCRIPT_DIR/client1.log" 2>&1 &
CLIENT1_PID=$!

# Poll Client 1 (Max 20s)
C1_CONNECTED=0
for i in {1..20}; do
    if grep -q "Connected successfully" "$SCRIPT_DIR/client1.log"; then
        C1_CONNECTED=1
        break
    fi
    sleep 1
done

if [ "$C1_CONNECTED" -eq 0 ]; then
    log_message "$LOGGING_LEVEL" "error" "Client 1 failed to connect (Timeout polling log)."
    log_message "$LOGGING_LEVEL" "debug" "Client 1 Log:"
    cat "$SCRIPT_DIR/client1.log"
    log_message "$LOGGING_LEVEL" "debug" "Server log snippet:"
    tail -n 50 "$LOG_FILE"
    kill "$SERVER_PID" 2>/dev/null
    exit 1
fi

# 6. Connect Client 2 (Should be rejected)
log_message "$LOGGING_LEVEL" "info" "Starting Client 2 (Should fail)..."
python3 -u "$WORKSPACE_DIR/scripts/check_connection.py" "0.0.0.0" "$BROKER_PORT" "c2" 60 > "$SCRIPT_DIR/client2.log" 2>&1 &
Client2_PID=$!

# Wait for Client 2 attempt (It might connect then disconnect, or fail connect)
# We wait 5s to allow connection attempt
sleep 5

# Check if Client 1 is still alive
if ! kill -0 "$CLIENT1_PID" 2>/dev/null; then
    log_message "$LOGGING_LEVEL" "error" "Client 1 disconnected prematurely!"
    cat "$SCRIPT_DIR/client1.log"
    STATUS=1
else
    # Check if Client 2 is still running
    # If limit is enforced, broker should close connection, causing python script to raise exception and exit.
    if kill -0 "$CLIENT2_PID" 2>/dev/null; then
        log_message "$LOGGING_LEVEL" "error" "Client 2 is still connected! Limit failed."
        log_message "$LOGGING_LEVEL" "debug" "Client 2 Log:"
        cat "$SCRIPT_DIR/client2.log"
        log_message "$LOGGING_LEVEL" "debug" "Server log snippet:"
        tail -n 20 "$LOG_FILE"
        kill "$CLIENT1_PID" 2>/dev/null
        kill "$CLIENT2_PID" 2>/dev/null
        STATUS=1
    else
        log_message "$LOGGING_LEVEL" "info" "Client 2 disconnected (Limit enforced)."
        STATUS=0
    fi
fi

# Cleanup
kill "$CLIENT1_PID" 2>/dev/null
kill "$CLIENT2_PID" 2>/dev/null
kill "$SERVER_PID" 2>/dev/null
rm -rf "$SCRIPT_DIR/conf" "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log" "$SCRIPT_DIR/client1.log" "$SCRIPT_DIR/client2.log"

exit $STATUS
