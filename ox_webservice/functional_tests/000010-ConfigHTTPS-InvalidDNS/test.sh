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

  # Create certs directory
  mkdir -p "$TEST_DIR/conf/certs"
  
  # Generate self-signed certificate
  openssl req -x509 -newkey rsa:4096 -keyout "$TEST_DIR/conf/certs/key.pem" -out "$TEST_DIR/conf/certs/cert.pem" -days 365 -nodes -subj "/CN=localhost" 2>/dev/null

  # Update config with absolute paths to certs
  # We use a temp config to avoid modifying the source constantly if we wanted to keep it clean, 
  # but for functional tests modifying the runtime config is fine or we create a runtime copy.
  # Let's create a runtime copy.
  cp "$TEST_DIR/conf/ox_webservice.yaml" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s|/var/repos/oxIDIZER/conf/certs/cert.pem|$TEST_DIR/conf/certs/cert.pem|g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s|/var/repos/oxIDIZER/conf/certs/key.pem|$TEST_DIR/conf/certs/key.pem|g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"

  # Create runtime config with dynamic port
  cp "$TEST_DIR/conf/ox_webservice.yaml" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s/port: 3000/port: $BASE_PORT/g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s/dependency_port: 3000/dependency_port: $BASE_PORT/g" "$TEST_DIR/conf/ox_webservice.runtime.yaml" # Just in case
  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Allow the server to start
  sleep 3

  # Curl the root page of the server using the default certificate WITH validation enabled.
  # We expect this to FAIL because of DNS/Certificate mismatch.
  curl --connect-timeout 30 --max-time 60 -i -s https://localhost:$BASE_PORT/ > "$TEST_DIR/curl_output.txt" 2>&1
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
