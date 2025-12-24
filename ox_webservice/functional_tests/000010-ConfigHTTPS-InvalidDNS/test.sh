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
  TEST_WORKSPACE_DIR=$(readlink -f "$TEST_DIR/../../../")

  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_DIR/ox_webservice.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Allow the server to start
  sleep 2

  # Curl the root page of the server using the default certificate WITH validation enabled.
  # We expect this to FAIL because of DNS/Certificate mismatch.
  curl -i -s https://localhost:3443/ > "$TEST_DIR/curl_output.txt" 2>&1
  CURL_EXIT_CODE=$?

  # Stop the server
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  # Output the log file
  if [ "$LOGGING_LEVEL" == "debug" ]; then
    log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
    cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
  fi

  # Check for correct initializing message in the log file (server started)
  if grep -q "Listening on" "$TEST_DIR/logs/ox_webservice.log"; then
      log_message "$LOGGING_LEVEL" "notice" "Found initializing message in log (server started)"
  else
      log_message "$LOGGING_LEVEL" "error" "Did not find initializing message in log (server failed to start)"
      log_message "$LOGGING_LEVEL" "error" "Test FAILED"
      exit $FAILED
  fi

  # Check validation results
  if [ "$CURL_EXIT_CODE" -ne 0 ]; then
    log_message "$LOGGING_LEVEL" "notice" "Curl failed as expected (Exit Code: $CURL_EXIT_CODE)."
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit $PASSED
  else
    log_message "$LOGGING_LEVEL" "error" "Curl SUCCEEDED unexpectedly (Exit Code: 0)."
    log_message "$LOGGING_LEVEL" "debug" "Curl output:"
    cat "$TEST_DIR/curl_output.txt" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
    log_message "$LOGGING_LEVEL" "error" "Test FAILED"
    exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
