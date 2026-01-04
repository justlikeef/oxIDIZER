#!/bin/bash

# Exit codes
PASSED=0
FAILED=255
SKIPPED=77

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../functional_tests/common

SCRIPTS_DIR=$1
TEST_LIBS_DIR=${2:-$DEFAULT_TEST_LIBS_DIR}
MODE=${3:-$DEFAULT_MODE}
LOGGING_LEVEL=${4:-$DEFAULT_LOGGING_LEVEL}
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

# Source the logging function
source "$TEST_LIBS_DIR/log_function.sh"

TEST_DIR=$(dirname "$(readlink -f "$0")")

if [ "$MODE" == "integrated" ]; then
  log_message "$LOGGING_LEVEL" "info" "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  # Define paths
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
  TEST_CONFIG_FILE="$TEST_WORKSPACE_DIR/conf/ox_webservice.yaml"

  log_message "$LOGGING_LEVEL" "info" "Regression Test: Verifying server starts with default config ($TEST_CONFIG_FILE)"

  # Create runtime config with dynamic port
  cp "$TEST_DIR/conf/ox_webservice.yaml" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s/port: 3000/port: $BASE_PORT/g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  # Start the server
  START_OUTPUT=$("$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_CONFIG_FILE" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR")
  
  log_message "$LOGGING_LEVEL" "debug" "$START_OUTPUT"
  
  if [ -f "$TEST_PID_FILE" ]; then
    SERVER_PID=$(cat "$TEST_PID_FILE")
    log_message "$LOGGING_LEVEL" "debug" "Read SERVER_PID: $SERVER_PID"
  else
    log_message "$LOGGING_LEVEL" "error" "PID file not found"
    exit $FAILED
  fi

  sleep 5 # Wait for startup

  if [ -n "$SERVER_PID" ] && ps -p "$SERVER_PID" > /dev/null; then
    log_message "$LOGGING_LEVEL" "notice" "Server started successfully. Regression test PASSED."
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
    exit $PASSED
  else
    log_message "$LOGGING_LEVEL" "error" "Server failed to start."
    if grep -q "missing field \`module_id\`" "$TEST_DIR/logs/ox_webservice.log"; then
        log_message "$LOGGING_LEVEL" "error" "Confirmed crash due to 'missing field module_id'."
    else
        log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
        cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
    fi
    exit $FAILED
  fi
fi

exit $FAILED
