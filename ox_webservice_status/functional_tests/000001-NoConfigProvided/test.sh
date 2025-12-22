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
  TEST_WORKSPACE_DIR=$(readlink -f "$TEST_DIR/../../../")
  mkdir -p "$TEST_DIR/logs"

  # Determine target (debug by default)
  TARGET="debug"
  SERVER_BIN="$TEST_WORKSPACE_DIR/target/$TARGET/ox_webservice"
  export LD_LIBRARY_PATH="$TEST_WORKSPACE_DIR/target/$TARGET"

  if [ ! -f "$SERVER_BIN" ]; then
      log_message "$LOGGING_LEVEL" "error" "Server binary not found at $SERVER_BIN"
      exit $FAILED
  fi

  # Cleanup port 3000 (and 8080 if default?)
  fuser -k 3000/tcp || true
  fuser -k 8080/tcp || true
  sleep 1
  
  # Temporarily move generic ox_webservice.yaml if it exists to prevent default loading
  MOVED_CONFIG=0
  if [ -f "$TEST_WORKSPACE_DIR/ox_webservice.yaml" ]; then
      mv "$TEST_WORKSPACE_DIR/ox_webservice.yaml" "$TEST_WORKSPACE_DIR/ox_webservice.yaml.bak"
      MOVED_CONFIG=1
  fi
  
  # Ensure cleanup on exit
  restore_config() {
      if [ $MOVED_CONFIG -eq 1 ]; then
          mv "$TEST_WORKSPACE_DIR/ox_webservice.yaml.bak" "$TEST_WORKSPACE_DIR/ox_webservice.yaml"
      fi
  }
  trap restore_config EXIT

  # Run server without -c argument
  # It should try to load default ox_webservice.yaml and fail
  OUTPUT=$("$SERVER_BIN" run 2>&1)
  
  # Check output for failure to load config
  if echo "$OUTPUT" | grep -q "Failed to load configuration" || \
     echo "$OUTPUT" | grep -q "Configuration file not found" || \
     echo "$OUTPUT" | grep -q "No such file or directory"; then
      log_message "$LOGGING_LEVEL" "info" "Test PASSED: Server failed to load default config."
      exit $PASSED
  else
      log_message "$LOGGING_LEVEL" "error" "Test FAILED: Server did not fail as expected. Output:"
      log_message "$LOGGING_LEVEL" "error" "$OUTPUT"
      exit $FAILED
  fi
fi
exit $FAILED
