#!/bin/bash
# 01_basic_rendering/test.sh

SUPPORT_SCRIPTS_DIR=$1
TEST_LIBS_DIR=$2
RUNNING_MODE=$3
LOGGING_LEVEL=$4
TARGET=$5
PORTS_STR=$6

# Get the first port
PORT=$(echo $PORTS_STR | cut -d' ' -f1)

TEST_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR=$(dirname "$(dirname "$(dirname "$(dirname "$TEST_DIR")")")") # ox_forms/ox_forms_server/functional_tests/01... -> workspace

# Source common libs

# Create temp config files
TEMP_CONFIG_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_CONFIG_DIR"' EXIT

SERVER_CONFIG="$TEMP_CONFIG_DIR/ox_webservice.yaml"
FORMS_FILE="$TEST_DIR/forms.json"
MODULE_CONFIG="$TEMP_CONFIG_DIR/ox_forms_server.yaml"

# Symlink forms file to temp dir to ensure accessibility if needed, or just use absolute path
# We'll use absolute path in the config

# generate server config
cat <<EOF > "$SERVER_CONFIG"
servers:
  - id: "test_server"
    protocol: "http"
    port: $PORT
    bind_address: "127.0.0.1"
    hosts:
      - name: "*"

log4rs_config: "$WORKSPACE_DIR/conf/log4rs.yaml"

logging:
  level: "debug"

pipeline:
  phases:
    - Content: "ox_pipeline_router"

modules:
  - id: "forms_server"
    name: "ox_forms_server"
    phase: Content
    params:
      forms_file: "$FORMS_FILE"

routes:
  - url: "/forms/.*"
    match_type: "regex"
    module_id: "forms_server"
    phase: Content
EOF

# Start Server
# We use the workspace start_server.sh but override config
SERVER_LOG="$TEMP_CONFIG_DIR/server.log"
SERVER_PID_FILE="$TEMP_CONFIG_DIR/server.pid"

"$WORKSPACE_DIR/scripts/start_server.sh" "$LOGGING_LEVEL" "$TARGET" "$SERVER_CONFIG" "$SERVER_LOG" "$SERVER_PID_FILE" "$WORKSPACE_DIR"

# Wait for server
sleep 2

# Test Request
# The module currently hardcodes looking for "server_test_form" or the first one.
# Our forms.json has id "server_test_form".

RESPONSE=$(curl -s -v --connect-timeout 5 "http://127.0.0.1:$PORT/forms/render" 2>&1)

# Verify
if echo "$RESPONSE" | grep -q "id=\"server_test_form\""; then
    echo "Passed: Form ID found."
else
    echo "Failed: Form ID not found."
    echo "Response: $RESPONSE"
    exit 1
fi

if echo "$RESPONSE" | grep -q "Test Field Label"; then
    echo "Passed: Field label found."
else
    echo "Failed: Field label not found."
    echo "Response: $RESPONSE"
    exit 1
fi

# Cleanup happens in trap/start_server logic (start_server kills prev pid file if same, but here we used unique pid file)
# explicitly kill
if [ -f "$SERVER_PID_FILE" ]; then
    kill $(cat "$SERVER_PID_FILE") 2>/dev/null
fi

exit 0
