#!/bin/bash

# Exit codes
PASSED=1
FAILED=255
SKIPPED=0

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../functional_tests/common

SCRIPTS_DIR=$1
TEST_LIBS_DIR=${2:-$DEFAULT_TEST_LIBS_DIR}
MODE=${3:-$DEFAULT_MODE}
LOGGING_LEVEL=${4:-$DEFAULT_LOGGING_LEVEL}

source "$TEST_LIBS_DIR/log_function.sh"
TEST_DIR=$(dirname "$(readlink -f "$0")")

if [ "$MODE" == "integrated" ]; then
  log_message "$LOGGING_LEVEL" "info" "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR=$(readlink -f "$TEST_DIR/../../../")
  mkdir -p "$TEST_DIR/logs"

  # Cleanup port 3000
  fuser -k 3000/tcp || true
  sleep 1

  # Start the server with non-existent config
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_DIR/non_existent.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  sleep 2

  if grep -q "Configuration file not found" "$TEST_DIR/logs/ox_webservice.log" || \
     grep -q "No such file or directory" "$TEST_DIR/logs/ox_webservice.log" || \
     grep -q "Failed to load configuration" "$TEST_DIR/logs/ox_webservice.log"; then
      log_message "$LOGGING_LEVEL" "info" "Test PASSED: Server failed to load missing config."
      exit $PASSED
  else
       if [ -f "$TEST_PID_FILE" ]; then
           PID=$(cat "$TEST_PID_FILE")
           if ps -p $PID > /dev/null; then
               log_message "$LOGGING_LEVEL" "error" "Test FAILED: Server started unexpectedly."
               "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
               exit $FAILED
           fi
       fi
       # If not running and no error log?
       log_message "$LOGGING_LEVEL" "info" "Test PASSED: Server exited."
       exit $PASSED
  fi
fi
exit $FAILED
