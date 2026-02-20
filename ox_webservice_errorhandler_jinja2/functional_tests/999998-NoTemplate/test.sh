#!/bin/bash

# Exit codes
PASSED=0
FAILED=255
SKIPPED=77

# Parameters
DEFAULT_LOGGING_LEVEL="info"
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}
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
  mkdir -p "$TEST_DIR/conf"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
  
  # Error Handler Config (point to empty/invalid template root)
  cat <<EOF > "$TEST_DIR/conf/eh_config.json"
{
  "content_root": "$TEST_DIR/conf",
  "debug_force_status": 404
}
EOF

  # Generate helper configs
  cat <<EOF > "$TEST_DIR/conf/log4rs.yaml"
appenders:
  stdout:
    kind: console
    encoder:
      pattern: "{d} {l} {t} - {m}{n}"
  file:
    kind: file
    path: "$TEST_DIR/logs/ox_webservice.log"
    encoder:
      pattern: "{d} {l} {t} - {m}{n}"
root:
  level: debug
  appenders:
    - stdout
    - file
EOF

  # Runtime Config
  cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_DIR/conf/log4rs.yaml"
modules:
  - id: eh_module
    name: ox_webservice_errorhandler_jinja2
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_errorhandler_jinja2.so"
    params:
      config_file: "$TEST_DIR/conf/eh_config.json"
  - id: ox_pipeline_router
    name: ox_pipeline_router
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_pipeline_router.so"
servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "0.0.0.0"
    hosts:
      - name: "localhost"
pipeline:
  phases:
    - Content: "ox_pipeline_router"
    - ErrorHandling: default
routes:
  - url: ".*"
    module_id: "eh_module"
    phase: Content
EOF

  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Allow the server to start
  sleep 3

  # Curl the non-existent page
  HTTP_STATUS=$(curl --connect-timeout 30 --max-time 60 -s -o /dev/null -w "%{http_code}" http://localhost:$BASE_PORT/doesnotexist.html)
  CURL_OUTPUT=$(curl --connect-timeout 30 --max-time 60 -s http://localhost:$BASE_PORT/doesnotexist.html)

  # Stop the server
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  # Check for correct  message in the log file
  # Expect fallback message for 404
  if grep -q "No specific error template found for status 404" "$TEST_DIR/logs/ox_webservice.log"; then
      log_message "$LOGGING_LEVEL" "notice" "Found 'No specific error template' message in log"
  else
      log_message "$LOGGING_LEVEL" "error" "Did not find 'No specific error template' message in log"
      log_message "$LOGGING_LEVEL" "error" "Test FAILED"
      exit $FAILED
  fi

  # Check the output. Should be default 404 text from error handler fallback
  # "404 Not Found"
  if [ "$HTTP_STATUS" -eq 404 ] && echo "$CURL_OUTPUT" | grep -q "404 Not Found"; then
    log_message "$LOGGING_LEVEL" "notice" "Found 404 status code and correct error message in body."

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
      cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done

      log_message "$LOGGING_LEVEL" "debug" "Curl Status: $HTTP_STATUS"
      log_message "$LOGGING_LEVEL" "debug" "Curl Output:"
      log_message "$LOGGING_LEVEL" "debug" "$CURL_OUTPUT"
    fi

    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit $PASSED
  else
    log_message "$LOGGING_LEVEL" "error" "Did not find 404 status and/or correct body."
    log_message "$LOGGING_LEVEL" "error" "Expected Status: 404, Actual: $HTTP_STATUS"
    log_message "$LOGGING_LEVEL" "error" "Expected Body: '404 Not Found', Actual: '$CURL_OUTPUT'"

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
      cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
    fi

    log_message "$LOGGING_LEVEL" "debug" "Curl Status: $HTTP_STATUS"
    log_message "$LOGGING_LEVEL" "debug" "Curl Output:"
    log_message "$LOGGING_LEVEL" "debug" "$CURL_OUTPUT"

    log_message "$LOGGING_LEVEL" "error" "Test FAILED"   
    exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
