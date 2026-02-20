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

  # Create invalid config
  cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"
modules: [
# Missing closing bracket
EOF

  # Start the server and capture the output
  START_OUTPUT=$("$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR")
  log_message "$LOGGING_LEVEL" "debug" "$START_OUTPUT"
  
  # The PID is now read directly from the PID file created by start_server.sh
  if [ -f "$TEST_PID_FILE" ]; then
    SERVER_PID=$(cat "$TEST_PID_FILE")
    log_message "$LOGGING_LEVEL" "debug" "Read SERVER_PID from file: $SERVER_PID"
  else
    SERVER_PID=""
    log_message "$LOGGING_LEVEL" "error" "PID file not found: $TEST_PID_FILE"
  fi

  # Allow the server to start
  sleep 2

  # Check if the process is running
  if [ -n "$SERVER_PID" ] && ps -p "$SERVER_PID" > /dev/null; then
    log_message "$LOGGING_LEVEL" "debug" "Server process with PID $SERVER_PID is running."
    # Stop the server
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
      cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
    fi

    log_message "$LOGGING_LEVEL" "error" "Test FAILED"
    exit $FAILED

  else
    log_message "$LOGGING_LEVEL" "notice" "Server process with PID $SERVER_PID is not running (or PID was empty)."

    # Check for panics in the log file
    if grep -q "panic" "$TEST_DIR/logs/ox_webservice.log"; then
        log_message "$LOGGING_LEVEL" "error" "Panic detected in log file."
        log_message "$LOGGING_LEVEL" "error" "Test FAILED"
        exit $FAILED
    else
        log_message "$LOGGING_LEVEL" "debug" "No panics detected in log file."
    fi

    # Check for correct error message in the log file
    if grep -q "Error parsing file" "$TEST_DIR/logs/ox_webservice.log"; then
        log_message "$LOGGING_LEVEL" "notice" "Found config deserialization error in log"
    else
        log_message "$LOGGING_LEVEL" "error" "Did not find error in log"
        log_message "$LOGGING_LEVEL" "error" "Test FAILED"
        exit $FAILED
    fi

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
      cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
    fi

    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit $PASSED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
