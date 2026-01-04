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
  # Define paths
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"

  # Dynamic Config Generation
  mkdir -p "$TEST_DIR/conf"
  cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: ping_module
    name: ox_webservice_ping
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_ping.so"
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

routes:
  - url: "^/ping"
    module_id: "ping_module"
    phase: Content
    priority: 100
EOF
  # End Dynamic Config
  
  # Define paths for the new parameters
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"

  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Allow the server to start
  sleep 2

  FAILURES=0

  # 1. Test Default HTML
  log_message "$LOGGING_LEVEL" "info" "Testing Default (HTML)..."
  RESP=$(curl --connect-timeout 30 --max-time 60 -i -s http://localhost:$BASE_PORT/ping)
  if echo "$RESP" | grep -i -q "content-type: text/html"; then
      if echo "$RESP" | grep -q "result: pong"; then
          log_message "$LOGGING_LEVEL" "info" "Default HTML test passed."
      else
          log_message "$LOGGING_LEVEL" "error" "Default HTML test body failed."
          FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "error" "Default HTML test headers failed."
      FAILURES=$((FAILURES + 1))
  fi

  # 2. Test JSON via Header
  log_message "$LOGGING_LEVEL" "info" "Testing JSON (Accept Header)..."
  RESP=$(curl --connect-timeout 30 --max-time 60 -i -s -H "Accept: application/json" http://localhost:$BASE_PORT/ping)
  if echo "$RESP" | grep -i -q "content-type: application/json"; then
      if echo "$RESP" | grep -q '"result":"pong"'; then
          log_message "$LOGGING_LEVEL" "info" "JSON Header test passed."
      else
          log_message "$LOGGING_LEVEL" "error" "JSON Header test body failed."
          FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "error" "JSON Header test headers failed."
      FAILURES=$((FAILURES + 1))
  fi

  # 3. Test JSON via Query
  log_message "$LOGGING_LEVEL" "info" "Testing JSON (Query Param)..."
  RESP=$(curl --connect-timeout 30 --max-time 60 -i -s http://localhost:$BASE_PORT/ping?format=json)
  if echo "$RESP" | grep -i -q "content-type: application/json"; then
      if echo "$RESP" | grep -q '"result":"pong"'; then
          log_message "$LOGGING_LEVEL" "info" "JSON Query test passed."
      else
          log_message "$LOGGING_LEVEL" "error" "JSON Query test body failed."
          FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "error" "JSON Query test headers failed."
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
