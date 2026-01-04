#!/bin/bash

# Exit codes
PASSED=0
FAILED=255

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../functional_tests/common

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
  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Allow the server to start
  sleep 2

  # Make a request to a non-existent file
  curl --connect-timeout 30 --max-time 60 -s http://localhost:$BASE_PORT/non_existent_file.html > /dev/null

  # Stop the server
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
  sync # Ensure logs are flushed to disk
  sleep 1

  # Check for correct  message in the log file
  if grep -q "ox_webservice_stream: File not found for path: /non_existent_file.html" "$TEST_DIR/logs/ox_webservice.log"; then
      log_message "$LOGGING_LEVEL" "notice" "Found 'File not found' message in log"
      log_message "$LOGGING_LEVEL" "info" "Test PASSED"
      exit $PASSED
  else
      log_message "$LOGGING_LEVEL" "error" "Did not find 'File not found' message in log"
      log_message "$LOGGING_LEVEL" "error" "Test FAILED"
      exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED