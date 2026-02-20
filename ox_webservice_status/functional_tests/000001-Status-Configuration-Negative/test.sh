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
TEST_LIBS_DIR=${2:-$DEFAULT_TEST_LIBS_DIR}
MODE=${3:-$DEFAULT_MODE}
LOGGING_LEVEL=${4:-$DEFAULT_LOGGING_LEVEL}
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

source "$TEST_LIBS_DIR/log_function.sh"
TEST_DIR=$(dirname "$(readlink -f "$0")")

if [ "$MODE" == "integrated" ]; then
  log_message "$LOGGING_LEVEL" "info" "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
  mkdir -p "$TEST_DIR/logs"

  # Cleanup
  fuser -k 3000/tcp || true
  sleep 1

  FAILURES=0

  # Case 1: Start with INVALID config file path provided in params
  # Dynamic Config
  mkdir -p "$TEST_DIR/conf"
  cat <<EOF > "$TEST_DIR/conf/ox_webservice_1.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"
modules:
  - id: status_module
    name: ox_webservice_status
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_status.so"
    config_file: "/non/existent/path/to/status.yaml"
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
  - url: "^/status"
    module_id: "status_module"
    phase: Content
    priority: 100
EOF

  log_message "$LOGGING_LEVEL" "info" "Starting server with invalid module config path..."
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" "$TARGET" "$TEST_DIR/conf/ox_webservice_1.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
  sleep 4
  
  if [ -f "$TEST_PID_FILE" ] && kill -0 $(cat "$TEST_PID_FILE") 2>/dev/null; then
      RESP=$(curl --connect-timeout 30 --max-time 60 -s -H "Accept: application/json" http://localhost:$BASE_PORT/status)
      if echo "$RESP" | grep -q "/non/existent/path/to/status.yaml"; then
          log_message "$LOGGING_LEVEL" "info" "PASS: Module started and reported config path"
      else
          log_message "$LOGGING_LEVEL" "error" "FAIL: Module did not report expected config path. Got: $RESP"
          FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "info" "Server failed to start (expected behavior for invalid config). Checking logs..."
      if grep -q "No such file or directory" "$TEST_DIR/logs/ox_webservice.log" || grep -q "failed to load" "$TEST_DIR/logs/ox_webservice.log"; then
           log_message "$LOGGING_LEVEL" "info" "PASS: Found expected error message in logs."
      else
           log_message "$LOGGING_LEVEL" "error" "FAIL: Server failed but expected error message not found."
           cat "$TEST_DIR/logs/ox_webservice.log"
           FAILURES=$((FAILURES + 1))
      fi
  fi
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  # Case 2: Missing 'config_file' param
  cat <<EOF > "$TEST_DIR/conf/ox_webservice_2.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"
modules:
  - id: status_module
    name: ox_webservice_status
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_status.so"
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
  - url: "^/status"
    module_id: "status_module"
    phase: Content
    priority: 100
EOF
  
  log_message "$LOGGING_LEVEL" "info" "Starting server with missing config param..."
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" "$TARGET" "$TEST_DIR/conf/ox_webservice_2.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
  sleep 4

  if [ -f "$TEST_PID_FILE" ] && kill -0 $(cat "$TEST_PID_FILE") 2>/dev/null; then
      RESP=$(curl --connect-timeout 30 --max-time 60 -s -H "Accept: application/json" http://localhost:$BASE_PORT/status)
      if ! echo "$RESP" | grep -q "\"config_file\""; then
          log_message "$LOGGING_LEVEL" "info" "PASS: Module started and config_file key is absent"
      else
          log_message "$LOGGING_LEVEL" "error" "FAIL: content should be null but got: $RESP"
          FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "info" "Server failed to start. Checking logs..."
      if grep -q "missing field" "$TEST_DIR/logs/ox_webservice.log"; then
           log_message "$LOGGING_LEVEL" "info" "PASS: Found expected missing field error in logs."
      else
           log_message "$LOGGING_LEVEL" "error" "FAIL: Server failed but expected error message not found."
           cat "$TEST_DIR/logs/ox_webservice.log"
           FAILURES=$((FAILURES + 1))
      fi
  fi
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  # Case 3: Invalid 'config_file' param type (int)
  cat <<EOF > "$TEST_DIR/conf/ox_webservice_3.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"
modules:
  - id: status_module
    name: ox_webservice_status
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_status.so"
    config_file: 12345
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
  - url: "^/status"
    module_id: "status_module"
    phase: Content
    priority: 100
EOF

  log_message "$LOGGING_LEVEL" "info" "Starting server with invalid config param type..."
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" "$TARGET" "$TEST_DIR/conf/ox_webservice_3.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
  sleep 4

  if [ -f "$TEST_PID_FILE" ] && kill -0 $(cat "$TEST_PID_FILE") 2>/dev/null; then
      RESP=$(curl --connect-timeout 30 --max-time 60 -s -H "Accept: application/json" http://localhost:$BASE_PORT/status)
      if echo "$RESP" | grep -q "\"config_file\":12345"; then
          log_message "$LOGGING_LEVEL" "info" "PASS: Module started and reported config value (ignored by logic)"
      else
          log_message "$LOGGING_LEVEL" "error" "FAIL: content should be null but got: $RESP"
          FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "info" "Server failed to start. Checking logs..."
      if grep -q "invalid type" "$TEST_DIR/logs/ox_webservice.log"; then
           log_message "$LOGGING_LEVEL" "info" "PASS: Found expected invalid type error in logs."
      else
           log_message "$LOGGING_LEVEL" "error" "FAIL: Server failed but expected error message not found."
           cat "$TEST_DIR/logs/ox_webservice.log"
           FAILURES=$((FAILURES + 1))
      fi
  fi
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  if [ $FAILURES -eq 0 ]; then
      exit $PASSED
  else
      exit $FAILED
  fi
fi
exit $FAILED
