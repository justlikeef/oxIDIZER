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
  mkdir -p "$TEST_DIR/logs"

  # Cleanup port 3000
  fuser -k 3000/tcp || true
  sleep 1

  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_DIR/ox_webservice.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Allow the server to start
  sleep 4

  FAILURES=0

  # 1. Test Default HTML (No Header)
  log_message "$LOGGING_LEVEL" "info" "Testing Default (HTML)..."
  RESP=$(curl -i -s http://localhost:3000/status)
  # Should fallback to stream module, which returns default.html/index.html containing HTML
  if echo "$RESP" | grep -i -q "content-type: text/html"; then
      log_message "$LOGGING_LEVEL" "info" "Default HTML test passed."
  else
      log_message "$LOGGING_LEVEL" "error" "Default HTML test headers failed. Got:"
      echo "$RESP" | head -n 10
      FAILURES=$((FAILURES + 1))
  fi

  # 2. Test JSON via Header
  log_message "$LOGGING_LEVEL" "info" "Testing JSON (Accept Header)..."
  RESP_FILE="$TEST_DIR/resp_json.txt"
  curl -v -i -H "Accept: application/json" http://localhost:3000/status > "$RESP_FILE" 2>&1
  
  if grep -i -q "content-type: application/json" "$RESP_FILE"; then
      # Check for some JSON field like 'system_name' or 'uptime'
      if grep -q '"system_name"' "$RESP_FILE"; then
          log_message "$LOGGING_LEVEL" "info" "JSON Header test passed."
      else
          log_message "$LOGGING_LEVEL" "error" "JSON Header test body failed. Response:"
          cat "$RESP_FILE"
          FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "error" "JSON Header test content-type failed. Response:"
      cat "$RESP_FILE"
      FAILURES=$((FAILURES + 1))
  fi

  # Stop the server
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  if [ $FAILURES -eq 0 ]; then
      log_message "$LOGGING_LEVEL" "info" "Test PASSED"
      exit $PASSED
  else
      log_message "$LOGGING_LEVEL" "error" "Test FAILED with $FAILURES failures"
      exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
