#!/bin/bash
set -e

SUPPORT_SCRIPTS_DIR="$1"
TEST_LIBS_DIR="$2"
RUNNING_MODE="$3"
LOGGING_LEVEL="$4"
TARGET="$5"
PORTS_STR=${6:-"3005 3006 3007 3008 3009"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}
PORT2=${PORTS[1]}

source "$TEST_LIBS_DIR/log_function.sh"

SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR="/var/repos/oxIDIZER"

SERVER_START_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
SERVER_STOP_SCRIPT="$WORKSPACE_DIR/scripts/stop_server.sh"

log_message "$LOGGING_LEVEL" "info" "Starting Test: 200002-ConflictManagement"

mkdir -p "$SCRIPT_DIR/conf/datastores" "$SCRIPT_DIR/logs"
rm -f "$SCRIPT_DIR/conf/datastores"/*

FAILURES=0

# --- Test 1: on_content_conflict=skip module responds to /data_sources ---
log_message "$LOGGING_LEVEL" "info" "1. Testing data_source_manager with on_content_conflict=skip"

cat > "$SCRIPT_DIR/conf/ox_webservice.skip.yaml" <<EOF
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"
modules:
  - id: data_source_manager_skip
    name: ox_persistence_datasource_manager
    path: "$WORKSPACE_DIR/target/$TARGET/libox_persistence_datasource_manager.so"
    params:
      data_sources_dir: "$SCRIPT_DIR/conf/datastores"
      on_content_conflict: "skip"
servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "127.0.0.1"
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
  - url: "^/data_sources(.*)?$"
    module_id: "data_source_manager_skip"
    priority: 100
EOF

PID_FILE1="$SCRIPT_DIR/ox_webservice_skip.pid"
LOG_FILE1="$SCRIPT_DIR/logs/ox_webservice_skip.log"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$SCRIPT_DIR/conf/ox_webservice.skip.yaml" "$LOG_FILE1" "$PID_FILE1" "$WORKSPACE_DIR" > "$SCRIPT_DIR/logs/start_skip.log" 2>&1 &
sleep 3

STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:$BASE_PORT/data_sources")
if [ "$STATUS" = "200" ]; then
    log_message "$LOGGING_LEVEL" "info" "Skip module OK (status $STATUS)"
else
    log_message "$LOGGING_LEVEL" "error" "Skip module failed! Expected 200, got: $STATUS"
    cat "$LOG_FILE1" 2>/dev/null || true
    FAILURES=$((FAILURES + 1))
fi

if [ -f "$PID_FILE1" ]; then
    "$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE1" "$WORKSPACE_DIR" || true
fi
sleep 1

# --- Test 2: on_content_conflict=error module responds to /data_sources ---
log_message "$LOGGING_LEVEL" "info" "2. Testing data_source_manager with on_content_conflict=error"

cat > "$SCRIPT_DIR/conf/ox_webservice.error.yaml" <<EOF
log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"
modules:
  - id: data_source_manager_error
    name: ox_persistence_datasource_manager
    path: "$WORKSPACE_DIR/target/$TARGET/libox_persistence_datasource_manager.so"
    params:
      data_sources_dir: "$SCRIPT_DIR/conf/datastores"
      on_content_conflict: "error"
servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "127.0.0.1"
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
  - url: "^/data_sources(.*)?$"
    module_id: "data_source_manager_error"
    priority: 100
EOF

PID_FILE2="$SCRIPT_DIR/ox_webservice_error.pid"
LOG_FILE2="$SCRIPT_DIR/logs/ox_webservice_error.log"

"$SERVER_START_SCRIPT" "$LOGGING_LEVEL" "$TARGET" "$SCRIPT_DIR/conf/ox_webservice.error.yaml" "$LOG_FILE2" "$PID_FILE2" "$WORKSPACE_DIR" > "$SCRIPT_DIR/logs/start_error.log" 2>&1 &
sleep 3

STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:$BASE_PORT/data_sources")
if [ "$STATUS" = "200" ]; then
    log_message "$LOGGING_LEVEL" "info" "Error module OK (status $STATUS)"
else
    log_message "$LOGGING_LEVEL" "error" "Error module failed! Expected 200, got: $STATUS"
    cat "$LOG_FILE2" 2>/dev/null || true
    FAILURES=$((FAILURES + 1))
fi

if [ -f "$PID_FILE2" ]; then
    "$SERVER_STOP_SCRIPT" "$LOGGING_LEVEL" "$PID_FILE2" "$WORKSPACE_DIR" || true
fi

# Cleanup
if [ $FAILURES -eq 0 ]; then
    rm -rf "$SCRIPT_DIR/conf/datastores" "$SCRIPT_DIR/logs" \
        "$SCRIPT_DIR/conf/ox_webservice.skip.yaml" "$SCRIPT_DIR/conf/ox_webservice.error.yaml"
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    exit 0
else
    log_message "$LOGGING_LEVEL" "error" "Test FAILED with $FAILURES failures"
    exit 1
fi
