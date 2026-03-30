#!/bin/bash
set -e

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

source "$TEST_LIBS_DIR/log_function.sh"

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
SERVER_STOP_SCRIPT="$WORKSPACE_DIR/scripts/stop_server.sh"

log_message "$LOGGING_LEVEL" "info" "Starting Test: 200001-Routing (Package Manager)"

# Dynamic Config Generation
cat <<EOF > "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"
modules:
  - id: package_manager
    name: ox_package_manager
    path: "$WORKSPACE_DIR/target/$TARGET/libox_package_manager.so"
    params:
      config_file: "$WORKSPACE_DIR/ox_package_manager/conf/manager.yaml"
  - id: package_manager_stream
    name: ox_webservice_stream
    path: "$WORKSPACE_DIR/target/$TARGET/libox_webservice_stream.so"
    params:
      content_root: "$WORKSPACE_DIR/ox_package_manager/content/www/"
    on_content_conflict: skip
  - id: ox_pipeline_router
    name: ox_pipeline_router
    path: "$WORKSPACE_DIR/target/$TARGET/libox_pipeline_router.so"
    params:
      routes:
        - matcher:
            path: "^/packages/upload"
            headers:
              Method: POST
          module_id: "package_manager"
          priority: 450
        - matcher:
            path: "^/packages/(list|install)/?"
            headers:
              Accept: "application/json"
          module_id: "package_manager"
          priority: 450
        - matcher:
            path: "^/packages/(list|install)/?"
            query:
              format: "json"
          module_id: "package_manager"
          priority: 450
        - matcher:
            path: "^/packages/?(.*)$"
          module_id: "package_manager_stream"
          priority: 500
servers:
  - id: "default"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "127.0.0.1"
pipeline:
  phases:
    - Content: "ox_pipeline_router"
EOF

LOG_FILE="$SCRIPT_DIR/logs/ox_webservice.log"
PID_FILE="$SCRIPT_DIR/ox_webservice.pid"
mkdir -p "$SCRIPT_DIR/logs"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$SCRIPT_DIR/conf/ox_webservice.runtime.yaml" "$LOG_FILE" "$PID_FILE" "$WORKSPACE_DIR" > "$SCRIPT_DIR/start_script.log" 2>&1 &

sleep 5

FAILURES=0

log_message "$LOGGING_LEVEL" "info" "1. Testing Accept: application/json -> List"
RESP=$(curl --connect-timeout 30 --max-time 60 -s -H "Accept: application/json" http://127.0.0.1:$BASE_PORT/packages/list)
if echo "$RESP" | grep -q '"result":"success"'; then
    log_message "$LOGGING_LEVEL" "info" "PASS: List API (Header) OK"
else
    log_message "$LOGGING_LEVEL" "error" "FAIL: List API (Header) failed: $RESP"
    FAILURES=$((FAILURES + 1))
fi

log_message "$LOGGING_LEVEL" "info" "2. Testing ?format=json -> List"
RESP=$(curl --connect-timeout 30 --max-time 60 -s "http://127.0.0.1:$BASE_PORT/packages/list?format=json")
if echo "$RESP" | grep -q '"result":"success"'; then
    log_message "$LOGGING_LEVEL" "info" "PASS: List API (Query) OK"
else
    log_message "$LOGGING_LEVEL" "error" "FAIL: List API (Query) failed: $RESP"
    FAILURES=$((FAILURES + 1))
fi

log_message "$LOGGING_LEVEL" "info" "3. Testing Fallback -> HTML"
RESP=$(curl --connect-timeout 30 --max-time 60 -s http://127.0.0.1:$BASE_PORT/packages/)
if echo "$RESP" | grep -q "<title>Package Manager | oxIDIZER</title>"; then
    log_message "$LOGGING_LEVEL" "info" "PASS: Fallback HTML OK"
else
    log_message "$LOGGING_LEVEL" "error" "FAIL: Fallback HTML failed: $RESP"
    FAILURES=$((FAILURES + 1))
fi

"$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"
# rm -rf "$SCRIPT_DIR/logs" "$PID_FILE" "$SCRIPT_DIR/start_script.log" "$SCRIPT_DIR/conf/ox_webservice.active.yaml"

if [ $FAILURES -eq 0 ]; then
    exit 0
else
    exit 1
fi
