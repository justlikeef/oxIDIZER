#!/bin/bash
# repro_crash.sh
source "/var/repos/oxIDIZER/functional_tests/common/log_function.sh"
TEST_DIR=$(dirname "$(readlink -f "$0")")
TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
LOG_FILE="$TEST_DIR/repro_crash.log"
PID_FILE="$TEST_DIR/repro.pid"

# Start server using the main runtime config (preserving the user's environment)
# assuming it's safe to run briefly. If not, I should use a temp config.
# But I want to reproduce the USER'S crash.
# However, running the main server might conflict if it's already running (even if broken).
# I'll try to use a temp config closely mimicking the real one, or just try to start it.
# Given the user said "server seems to be crashing", it's likely not running now.
# But I'll use a temp env to be safe and capture logs cleanly.

mkdir -p "$TEST_DIR/logs" "$TEST_DIR/conf"

# Copy active configs to temp location to replicate exact state
cp -r "$TEST_WORKSPACE_DIR/conf" "$TEST_DIR/conf_copy"

# Helper to start server
function start_server() {
    export LD_LIBRARY_PATH="$TEST_WORKSPACE_DIR/target/debug:$LD_LIBRARY_PATH"
    "$TEST_WORKSPACE_DIR/target/debug/ox_webservice" -c "$TEST_DIR/conf_copy/ox_webservice.runtime.yaml" run \
        > "$LOG_FILE" 2>&1 &
    echo $! > "$PID_FILE"
    log_message "info" "Server started with PID $(cat $PID_FILE)"
}

# Cleanup
function cleanup() {
    if [ -f "$PID_FILE" ]; then
        kill $(cat "$PID_FILE") 2>/dev/null
        rm "$PID_FILE"
    fi
     rm -rf "$TEST_DIR/conf_copy"
}
trap cleanup EXIT

# 1. Start Server
start_server
sleep 5

# 2. Check if alive
if ! kill -0 $(cat "$PID_FILE") 2>/dev/null; then
    log_message "error" "Server crashed properly immediately!"
    cat "$LOG_FILE"
    exit 1
fi

# 3. Hit the problematic URL
log_message "info" "Hitting /packages/ ..."
curl -v "http://127.0.0.1:3000/packages/" >> "$LOG_FILE" 2>&1

sleep 2

# 4. Check if alive
if ! kill -0 $(cat "$PID_FILE") 2>/dev/null; then
    log_message "error" "Server CRASHED after request!"
    cat "$LOG_FILE"
    exit 1
else
    log_message "info" "Server survived."
fi
