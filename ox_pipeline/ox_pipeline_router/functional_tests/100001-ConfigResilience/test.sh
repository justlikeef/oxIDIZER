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
mkdir -p "$TEST_DIR/logs"

# Config Setup
# We need to simulate the bad config module.
# Copy default config structure
# Config Setup
mkdir -p "$TEST_DIR/conf/modules/active"

# Dynamic Config using Heredoc
cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"

merge:
  - "$TEST_DIR/conf/modules/active/"

servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "0.0.0.0"
    hosts:
      - name: "localhost"

pipeline:
  phases:
    - PreEarlyRequest: "ox_pipeline_router"
    - EarlyRequest: "ox_pipeline_router"
    - PostEarlyRequest: "ox_pipeline_router"
    - PreAuthentication: "ox_pipeline_router"
    - Authentication: "ox_pipeline_router"
    - PostAuthentication: "ox_pipeline_router"
    - PreAuthorization: "ox_pipeline_router"
    - Authorization: "ox_pipeline_router"
    - PostAuthorization: "ox_pipeline_router"
    - PreContent: "ox_pipeline_router"
    - Content: "ox_pipeline_router"
    - PostContent: "ox_pipeline_router"
    - PreAccounting: "ox_pipeline_router"
    - Accounting: "ox_pipeline_router"
    - PostAccounting: "ox_pipeline_router"
    - PreErrorHandling: "ox_pipeline_router"
    - ErrorHandling: "ox_pipeline_router"
    - PostErrorHandling: "ox_pipeline_router"
    - PreLateRequest: "ox_pipeline_router"
    - LateRequest: "ox_pipeline_router"
    - PostLateRequest: "ox_pipeline_router"
EOF

# Create BAD config
BAD_CONFIG_FILE="$TEST_DIR/conf/modules/active/broken_test_module.yaml"
echo "modules:" > "$BAD_CONFIG_FILE"
echo "  - id: 'broken'" >> "$BAD_CONFIG_FILE"
echo "    path: \"/broken/\?\"" >> "$BAD_CONFIG_FILE" # Invalid escape for test

# Ensure main config includes this directory
# The default config likely has 'include: "modules/active/*.yaml"' or similar.
# We need to make sure it resolves relative to the config file or we update the path.
# Assuming relative path works from config location ($TEST_DIR/conf).

log_message "$LOGGING_LEVEL" "info" "Starting server with broken config..."
cat "$TEST_DIR/conf/ox_webservice.runtime.yaml"

"$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$LOG_FILE" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

sleep 5

# Check if running
if [ -f "$TEST_PID_FILE" ] && kill -0 $(cat "$TEST_PID_FILE") 2>/dev/null; then
    log_message "$LOGGING_LEVEL" "info" "Server is running (PASS)"
else
    log_message "$LOGGING_LEVEL" "error" "Server crashed (FAIL)"
    if [ -f "$LOG_FILE" ]; then cat "$LOG_FILE"; fi
    exit 1
fi

# Check logs for expected error (handled by errorhandler or logs)
# Using grep on log file
# The previous test looked for "Failed to process included file.*Skipping"
if grep -q "Failed to process included file" "$LOG_FILE" || grep -q "Error loading.*Skipping" "$LOG_FILE"; then
    log_message "$LOGGING_LEVEL" "info" "Log contains expected error message (PASS)"
else
    log_message "$LOGGING_LEVEL" "error" "Log missing expected error message (FAIL)"
    echo "=== LOG OUTPUT ==="
    cat "$LOG_FILE"
    echo "=================="
    # Clean up
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
    exit 1
fi

"$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
rm -rf "$TEST_DIR/conf" "$TEST_DIR/logs" "$TEST_PID_FILE" 

exit 0
