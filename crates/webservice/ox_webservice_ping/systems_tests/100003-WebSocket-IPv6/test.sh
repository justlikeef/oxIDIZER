#!/bin/bash

# Exit codes
PASSED=0
FAILED=255
SKIPPED=77

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../../../tests/common

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

# Skip if IPv6 loopback is not available
if ! ip -6 addr show lo 2>/dev/null | grep -q "::1"; then
  log_message "$LOGGING_LEVEL" "info" "Skipping: IPv6 loopback (::1) not available on this system."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"

  mkdir -p "$TEST_DIR/conf" "$TEST_DIR/logs"
  cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
merge: "$TEST_WORKSPACE_DIR/conf/service/active/base.yaml"

log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: ping_module
    name: ox_webservice_ping
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_ping.so"
    phase: Content

servers:
  - protocol: http
    port: $BASE_PORT
    bind_address: "::1"
    hosts:
      - name: "localhost"

routes:
  - url: "^/ping"
    module_id: ping_module
    priority: 100
EOF

  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  sleep 2

  if [ ! -d "$TEST_DIR/venv" ]; then
    python3 -m venv "$TEST_DIR/venv"
    "$TEST_DIR/venv/bin/pip" install websockets --quiet
  fi

  PYTHON_OUTPUT=$("$TEST_DIR/venv/bin/python3" "$TEST_DIR/ws_ping_client.py" "$BASE_PORT" 2>&1)
  PYTHON_EXIT_CODE=$?

  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  if [ "$LOGGING_LEVEL" == "debug" ]; then
    log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
    cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
  fi

  if [ $PYTHON_EXIT_CODE -eq 0 ]; then
    log_message "$LOGGING_LEVEL" "notice" "WebSocket IPv6 ping test PASSED"
    log_message "$LOGGING_LEVEL" "debug" "Client output: $PYTHON_OUTPUT"
    exit $PASSED
  else
    log_message "$LOGGING_LEVEL" "error" "WebSocket IPv6 ping test FAILED"
    log_message "$LOGGING_LEVEL" "error" "Client output: $PYTHON_OUTPUT"
    exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
