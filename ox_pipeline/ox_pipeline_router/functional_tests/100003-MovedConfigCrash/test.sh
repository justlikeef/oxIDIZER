#!/bin/bash
# Exit codes
PASSED=0
FAILED=255

SCRIPTS_DIR="/var/repos/oxIDIZER/scripts"
TEST_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

LOG_FILE="$TEST_DIR/start_attempt.log"
PID_FILE="$TEST_DIR/server.pid"

# Cleanup
rm -f "$LOG_FILE" "$PID_FILE"
killall ox_webservice 2>/dev/null
sleep 1

echo "Attempting to start server with global config..."
"$SCRIPTS_DIR/start_server.sh" "debug" "debug" "$WORKSPACE_DIR/conf/ox_webservice.runtime.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR"

sleep 3

# Check if server is running
if [ -f "$PID_FILE" ]; then
    SERVER_PID=$(cat "$PID_FILE")
    if ps -p "$SERVER_PID" > /dev/null; then
        echo "PASS: Server started successfully."
        kill "$SERVER_PID"
        exit $PASSED
    else
        echo "FAIL: Server crashed immediately."
        echo "--- Log Tail ---"
        tail -n 20 "$LOG_FILE"
        exit $FAILED
    fi
else
    echo "FAIL: Server failed to start (No PID file)."
    echo "--- Log Tail ---"
    tail -n 20 "$LOG_FILE"
    exit $FAILED
fi
