#!/bin/bash

# Arguments
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
TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"

if [ "$MODE" == "integrated" ]; then
    log_message "$LOGGING_LEVEL" "info" "Skipping in integrated mode"
    exit 77
fi

# Setup isolated environment
TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
LOG_FILE="$TEST_DIR/logs/ox_webservice.log"
mkdir -p "$TEST_DIR/logs" "$TEST_DIR/conf"

# Create a Dummy Config that points to ox_webservice_ping
# We will map /test to it twice with different priorities.
# Since ping module ignores params, we can reuse it. 
# ...Wait, we need to distinguish which route was hit.
# ox_webservice_stream is better because we can serve different content.

mkdir -p "$TEST_DIR/content/low_priority"
echo "LOW_PRIORITY_HIT" > "$TEST_DIR/content/low_priority/index.html"

mkdir -p "$TEST_DIR/content/high_priority"
echo "HIGH_PRIORITY_HIT" > "$TEST_DIR/content/high_priority/index.html"

cat <<EOF > "$TEST_DIR/conf/low_priority.yaml"
content_root: "$TEST_DIR/content/low_priority"
mimetypes_file: "$TEST_DIR/conf/mimetypes.yaml"
default_documents:
  - document: "index.html"
EOF

cat <<EOF > "$TEST_DIR/conf/high_priority.yaml"
content_root: "$TEST_DIR/content/high_priority"
mimetypes_file: "$TEST_DIR/conf/mimetypes.yaml"
default_documents:
  - document: "index.html"
EOF

echo "mimetypes: []" > "$TEST_DIR/conf/mimetypes.yaml"

cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"

modules:
  - id: low_priority_module
    name: ox_webservice_stream
    params:
      config_file: "$TEST_DIR/conf/low_priority.yaml"
  - id: high_priority_module
    name: ox_webservice_stream
    params:
      config_file: "$TEST_DIR/conf/high_priority.yaml"

servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "0.0.0.0"

pipeline:
  phases:
    - Content: "ox_pipeline_router"

routes:
  - url: "^/test/(.*)$"
    module_id: "low_priority_module"
    phase: Content
    priority: 10
  - url: "^/test/(.*)$"
    module_id: "high_priority_module"
    phase: Content
    priority: 100
EOF

# Start Server
"$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$LOG_FILE" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

sleep 5

# Verify
RESP=$(curl -s "http://127.0.0.1:$BASE_PORT/test/index.html")

if [[ "$RESP" == *"LOW_PRIORITY_HIT"* ]]; then
    log_message "$LOGGING_LEVEL" "info" "Hit Low Priority (10) - System is Ascending (Low=Best)"
    STATUS=0
elif [[ "$RESP" == *"HIGH_PRIORITY_HIT"* ]]; then
    log_message "$LOGGING_LEVEL" "error" "Hit High Priority (100) - System is Descending (High=Best)"
    STATUS=1
else
    log_message "$LOGGING_LEVEL" "error" "Hit Unknown: '$RESP'"
    if [ -f "$LOG_FILE" ]; then
        echo "=== SERVER LOG ==="
        cat "$LOG_FILE"
        echo "=================="
    fi
    STATUS=1
fi

"$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
# rm -rf "$TEST_DIR/conf" "$TEST_DIR/logs" "$TEST_PID_FILE" "$TEST_DIR/content"

exit $STATUS
