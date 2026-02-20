#!/bin/bash
# Exit codes
PASSED=0
FAILED=255

SCRIPTS_DIR="/var/repos/oxIDIZER/scripts"
TEST_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

LOG_FILE="$TEST_DIR/start_attempt.log"
PID_FILE="$TEST_DIR/server.pid"

PORTS_STR=$6
if [ -z "$PORTS_STR" ]; then
    # Fallback for manual run
    PORT=8090
else
    # Take the first port
    PORT=$(echo $PORTS_STR | cut -d' ' -f1)
fi

CONFIG_FILE="$TEST_DIR/test_config.yaml"

DRIVERS_FILE="$TEST_DIR/drivers.yaml"
echo "[]" > "$DRIVERS_FILE"

# Generate config with correct port and ISOLATED routes
cat > "$CONFIG_FILE" <<EOF
log4rs_config: "conf/log4rs.yaml"

servers:
 - id: "default_http"
   protocol: "http"
   port: $PORT
   bind_address: "127.0.0.1"

pipeline:
 phases:
   - Content: "ox_pipeline_router"

modules:
  - id: "driver_manager"
    name: "ox_persistence_driver_manager"
    phase: Content
    params:
      drivers_file: "$DRIVERS_FILE"
  - id: "ox_pipeline_router"
    name: "ox_pipeline_router"
    params: {}

routes:
  - url: "^/drivers"
    headers:
      Accept: "application/json"
    module_id: "driver_manager"
  - url: "^/drivers"
    query:
      format: "json"
    module_id: "driver_manager"
EOF

# Cleanup
rm -f "$LOG_FILE" "$PID_FILE"
killall ox_webservice 2>/dev/null
sleep 1

echo "Starting server on port $PORT..."
"$SCRIPTS_DIR/start_server.sh" "debug" "debug" "$CONFIG_FILE" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR"

sleep 3

if [ ! -f "$PID_FILE" ]; then
    echo "FAIL: Server failed to start."
    echo "--- Log Tail ---"
    tail -n 10 "$LOG_FILE"
    exit $FAILED
fi

SERVER_PID=$(cat "$PID_FILE")

# Helper to kill logic
cleanup_and_exit() {
    kill "$SERVER_PID"
    rm -f "$CONFIG_FILE" "$DRIVERS_FILE"
    exit $1
}

# 1. Plain Request (Should 404 or 500 or just not be 200 from driver manager)
echo "Testing Plain Request (Expect Skip)..."
CODE=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" http://127.0.0.1:$PORT/drivers)
if [ "$CODE" == "200" ]; then
    echo "FAIL: Plain request returned 200 (Should be skipped/filtered)"
    cleanup_and_exit $FAILED
else
    echo "PASS: Plain request code: $CODE"
fi

# 2. Header Request (Expect 200)
echo "Testing Header Request (Expect 200)..."
CODE=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" -H "Accept: application/json" http://127.0.0.1:$PORT/drivers)
if [ "$CODE" != "200" ]; then
    echo "FAIL: Header request failed. Code: $CODE"
    echo "--- Server Log ---"
    tail -n 20 "$LOG_FILE"
    cleanup_and_exit $FAILED
fi
echo "PASS: Header request success."

# 3. Query Request (Expect 200)
echo "Testing Query Request (Expect 200)..."
CODE=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:$PORT/drivers?format=json")
if [ "$CODE" != "200" ]; then
    echo "FAIL: Query request failed. Code: $CODE"
    cleanup_and_exit $FAILED
fi
echo "PASS: Query request success."

cleanup_and_exit $PASSED
