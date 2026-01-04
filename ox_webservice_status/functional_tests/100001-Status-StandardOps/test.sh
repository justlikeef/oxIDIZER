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
  # Dynamic Config Generation
  mkdir -p "$TEST_DIR/conf"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
  cat <<EOF > "$TEST_DIR/conf/mimetypes.yaml"
mimetypes:
  - url: '.*\\.html$'
    mimetype: "text/html"
  - url: '.*\\.css$'
    mimetype: "text/css"
  - url: '.*\\.js$'
    mimetype: "application/javascript"
EOF

  cat <<EOF > "$TEST_DIR/conf/stream_config.json"
{
  "content_root": "$TEST_WORKSPACE_DIR/ox_webservice_status/content/www",
  "mimetypes_file": "$TEST_DIR/conf/mimetypes.yaml",
  "default_documents": [
    { "document": "index.html" }
  ]
}
EOF

  cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"
modules:
  - id: status_module
    name: ox_webservice_status
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_status.so"
  - id: stream_module
    name: ox_webservice_stream
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_webservice_stream.so"
    params:
      config_file: "$TEST_DIR/conf/stream_config.json"
      on_content_conflict: skip
  - id: ox_pipeline_router
    name: ox_pipeline_router
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_pipeline_router.so"
    params:
      routes:
        - matcher:
            path: "^/status"
          module_id: "status_module"
          priority: 100
        - matcher:
            path: "^/status(/.*)?"
          module_id: "stream_module"
          priority: 90
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
EOF

  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  mkdir -p "$TEST_DIR/logs"

  # Cleanup
  fuser -k $BASE_PORT/tcp || true
  sleep 1

  # Start server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" "$TARGET" "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
  sleep 4

  FAILURES=0

  # 1. Fallback HTML
  log_message "$LOGGING_LEVEL" "info" "1. Testing fallback HTML..."
  RESP=$(curl --connect-timeout 30 --max-time 60 -s -i http://localhost:$BASE_PORT/status)
  if echo "$RESP" | grep -i -q "Content-Type: text/html"; then
      log_message "$LOGGING_LEVEL" "info" "PASS: HTML returned by default"
  else
      log_message "$LOGGING_LEVEL" "error" "FAIL: Content-type not html"
      echo "$RESP" | head -n 5
      FAILURES=$((FAILURES + 1))
  fi

  # 2. JSON Header
  log_message "$LOGGING_LEVEL" "info" "2. Testing Accept: application/json..."
  RESP_JSON=$(curl --connect-timeout 30 --max-time 60 -s -i -H "Accept: application/json" http://localhost:$BASE_PORT/status)
  if echo "$RESP_JSON" | grep -i -q "Content-Type: application/json"; then
      if echo "$RESP_JSON" | grep -q "\"host_name\""; then
           log_message "$LOGGING_LEVEL" "info" "PASS: JSON returned with accept header"
      else
           log_message "$LOGGING_LEVEL" "error" "FAIL: JSON body missing expected keys"
           FAILURES=$((FAILURES + 1))
      fi
  else
      log_message "$LOGGING_LEVEL" "error" "FAIL: Content-type not json"
      FAILURES=$((FAILURES + 1))
  fi

  # 3. JSON Query
  log_message "$LOGGING_LEVEL" "info" "3. Testing ?format=json..."
  RESP_QUERY=$(curl --connect-timeout 30 --max-time 60 -s -i "http://localhost:$BASE_PORT/status?format=json")
  if echo "$RESP_QUERY" | grep -i -q "Content-Type: application/json"; then
      log_message "$LOGGING_LEVEL" "info" "PASS: JSON returned with format query"
  else
      log_message "$LOGGING_LEVEL" "error" "FAIL: Content-type not json"
      FAILURES=$((FAILURES + 1))
  fi

  # 4. Static Asset
  log_message "$LOGGING_LEVEL" "info" "4. Testing static asset /status/index.html..."
  RESP_ASSET=$(curl --connect-timeout 30 --max-time 60 -s -i http://localhost:$BASE_PORT/status/index.html)
  if echo "$RESP_ASSET" | grep -i -q "Content-Type: text/html"; then
      log_message "$LOGGING_LEVEL" "info" "PASS: Explicit static file returned"
  else
      log_message "$LOGGING_LEVEL" "error" "FAIL: Static file check failed"
      FAILURES=$((FAILURES + 1))
  fi

  # Stop server
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  if [ $FAILURES -eq 0 ]; then
      exit $PASSED
  else
      exit $FAILED
  fi
fi
exit $FAILED
