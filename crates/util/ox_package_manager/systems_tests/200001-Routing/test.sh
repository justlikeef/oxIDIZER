#!/bin/bash

SCRIPTS_DIR=$1
TEST_LIBS_DIR=$2
MODE=$3
LOGGING_LEVEL=$4
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

source "$TEST_LIBS_DIR/log_function.sh"

TEST_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

if [ "$MODE" == "integrated" ]; then
    log_message "$LOGGING_LEVEL" "info" "Skipping in integrated mode"
    exit 77
fi

TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
LOG_FILE="$TEST_DIR/logs/ox_webservice.log"
STAGING_DIR="/tmp/ox_test_staging_$$"
mkdir -p "$TEST_DIR/logs" "$TEST_DIR/conf"

# Generate stream config
cat <<EOF > "$TEST_DIR/conf/stream.yaml"
content_root: "$WORKSPACE_DIR/crates/util/ox_package_manager/content/www/"
mimetypes_file: "$WORKSPACE_DIR/conf/mimetypes.yaml"
default_documents:
  - document: "index.html"
on_content_conflict: "skip"
EOF

# Generate runtime config
cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: package_manager
    name: ox_package_manager
    path: "$WORKSPACE_DIR/target/$TARGET/libox_package_manager.so"
    staging_directory: "$STAGING_DIR"

  - id: package_manager_stream
    name: ox_webservice_stream
    path: "$WORKSPACE_DIR/target/$TARGET/libox_webservice_stream.so"
    config_file: "$TEST_DIR/conf/stream.yaml"
    on_content_conflict: "skip"

servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "0.0.0.0"
    hosts:
      - name: "localhost"

workflow:
  name: "ox_webservice"
  stages:
    - name: Content
      runner: sequential
      plugins:
        - name: ox_webservice_router
      on_error: continue

routes:
  - url: "^/packages/?(.*)?$"
    headers:
      Accept: "application/json"
    module_id: "package_manager"
    priority: 499
  - url: "^/packages/?(.*)?$"
    query:
      format: "json"
    module_id: "package_manager"
    priority: 499
  - url: "^/packages/?(.*)?$"
    module_id: "package_manager_stream"
    priority: 500
EOF

mkdir -p "$STAGING_DIR"

"$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$LOG_FILE" \
    "$TEST_PID_FILE" \
    "$WORKSPACE_DIR"

sleep 5

if [ ! -f "$TEST_PID_FILE" ] || ! kill -0 $(cat "$TEST_PID_FILE") 2>/dev/null; then
    log_message "$LOGGING_LEVEL" "error" "Server failed to start"
    if [ -f "$LOG_FILE" ]; then cat "$LOG_FILE"; fi
    exit 1
fi

FAILURES=0

log_message "$LOGGING_LEVEL" "info" "1. Testing Accept: application/json -> JSON List"
RESP=$(curl --connect-timeout 30 --max-time 60 -s -H "Accept: application/json" "http://127.0.0.1:$BASE_PORT/packages/list")
if echo "$RESP" | grep -q '"result":"success"'; then
    log_message "$LOGGING_LEVEL" "info" "PASS: List API (Accept header) OK"
else
    log_message "$LOGGING_LEVEL" "error" "FAIL: List API (Accept header) failed: $RESP"
    FAILURES=$((FAILURES + 1))
fi

log_message "$LOGGING_LEVEL" "info" "2. Testing ?format=json -> JSON List"
RESP=$(curl --connect-timeout 30 --max-time 60 -s "http://127.0.0.1:$BASE_PORT/packages/list?format=json")
if echo "$RESP" | grep -q '"result":"success"'; then
    log_message "$LOGGING_LEVEL" "info" "PASS: List API (query param) OK"
else
    log_message "$LOGGING_LEVEL" "error" "FAIL: List API (query param) failed: $RESP"
    FAILURES=$((FAILURES + 1))
fi

log_message "$LOGGING_LEVEL" "info" "3. Testing Fallback -> HTML"
RESP=$(curl --connect-timeout 30 --max-time 60 -s "http://127.0.0.1:$BASE_PORT/packages/")
if echo "$RESP" | grep -q "<title>Package Manager | oxIDIZER</title>"; then
    log_message "$LOGGING_LEVEL" "info" "PASS: Fallback HTML OK"
else
    log_message "$LOGGING_LEVEL" "error" "FAIL: Fallback HTML failed: $RESP"
    FAILURES=$((FAILURES + 1))
fi

"$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$WORKSPACE_DIR"

if [ $FAILURES -eq 0 ]; then
    rm -rf "$TEST_DIR/conf" "$TEST_DIR/logs" "$TEST_PID_FILE" "$STAGING_DIR"
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit 0
else
    log_message "$LOGGING_LEVEL" "error" "Test FAILED with $FAILURES failures"
    log_message "$LOGGING_LEVEL" "info" "Logs preserved in $TEST_DIR/logs"
    exit 1
fi
