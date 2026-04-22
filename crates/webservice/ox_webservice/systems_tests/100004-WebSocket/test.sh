#!/bin/bash

# Exit codes
PASSED=0
FAILED=255
SKIPPED=77

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../systems_tests/common

SCRIPTS_DIR=$1
# Use provided TEST_LIBS_DIR or the default
TEST_LIBS_DIR=${2:-$DEFAULT_TEST_LIBS_DIR}
# Use provided MODE or the default
MODE=${3:-$DEFAULT_MODE}
# Use provided LOGGING_LEVEL or the default
LOGGING_LEVEL=${4:-$DEFAULT_LOGGING_LEVEL}
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

# Source the logging function
source "$TEST_LIBS_DIR/log_function.sh"

# Get the directory of this script
TEST_DIR=$(dirname "$(readlink -f "$0")")

if [ "$MODE" == "integrated" ]; then
  log_message "$LOGGING_LEVEL" "info" "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  # Define paths for the new parameters
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"

  # Create runtime config with dynamic port
  cp "$TEST_DIR/conf/ox_webservice.yaml" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s/port: 3000/port: $BASE_PORT/g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s/dependency_port: 3000/dependency_port: $BASE_PORT/g" "$TEST_DIR/conf/ox_webservice.runtime.yaml" # Just in case
  
  # Fix hardcoded target paths
  sed -i "s|target/debug|target/$TARGET|g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"

  # Correct the server protocol to enable WebSockets and fix the module URI.
  sed -i 's/protocol: http/protocol: http-ws/' "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i '/- url: /s|\^ping/\?\$|\^/ping/\?\$|' "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  # Fix the module URI to match the client's request path
  sed -i 's|ping/?|/ping/?|g' "$TEST_DIR/conf/ox_webservice.runtime.yaml"

  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Wait for the server to be ready by polling the log file
  log_message "$LOGGING_LEVEL" "info" "Waiting for server to start..."
  ATTEMPTS=0
  MAX_ATTEMPTS=10 # Wait for 10 seconds max
  while ! grep -q "HTTP listener ready" "$TEST_DIR/logs/ox_webservice.log" 2>/dev/null; do
    if [ $ATTEMPTS -ge $MAX_ATTEMPTS ]; then
      log_message "$LOGGING_LEVEL" "error" "Server failed to start in time."
      cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "error" "  $line"; done
      "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
      exit $FAILED
    fi
    sleep 1
    ATTEMPTS=$((ATTEMPTS + 1))
  done
  log_message "$LOGGING_LEVEL" "info" "Server is ready."

  # Setup Virtual Env for WebSocket client
  if [ ! -d "$TEST_DIR/venv" ]; then
      python3 -m venv "$TEST_DIR/venv"
      "$TEST_DIR/venv/bin/pip" install websockets
  fi

  # Run the Python WebSocket Client
  # python3 "$TEST_DIR/ws_client.py" "$BASE_PORT"
  PYTHON_OUTPUT=$("$TEST_DIR/venv/bin/python3" "$TEST_DIR/ws_client.py" "$BASE_PORT" 2>&1)
  PYTHON_EXIT_CODE=$?

  # Stop the server
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  # Output the log file
  if [ "$LOGGING_LEVEL" == "debug" ]; then
    log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
    cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
  fi

  # Check validation
  if [ $PYTHON_EXIT_CODE -eq 0 ]; then
    log_message "$LOGGING_LEVEL" "notice" "WebSocket client reported SUCCESS"
    log_message "$LOGGING_LEVEL" "debug" "Client Output: $PYTHON_OUTPUT"
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit $PASSED
  else
    log_message "$LOGGING_LEVEL" "error" "WebSocket client reported FAILURE"
    log_message "$LOGGING_LEVEL" "debug" "Client Output: $PYTHON_OUTPUT"
    log_message "$LOGGING_LEVEL" "error" "Test FAILED"   
    exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
