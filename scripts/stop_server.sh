#!/bin/bash

# Determine the workspace directory dynamically
# Get the directory of the script
THIS_SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
# Assume the script is in a subdirectory of the workspace (e.g., /scripts)
DEFAULT_DERIVED_WORKSPACE_DIR=$(dirname "$THIS_SCRIPT_DIR")

# Parameters:
# 1: LOG_LEVEL (defaults to 'notice')
# 2: PID_FILE (defaults to $DEFAULT_DERIVED_WORKSPACE_DIR/ox_webservice.pid)
# 3: WORKSPACE_DIR (optional, for overriding derived path)

LOG_LEVEL=${1:-notice}
PID_FILE=${2:-$DEFAULT_DERIVED_WORKSPACE_DIR/ox_webservice.pid} # Use derived path as default
WORKSPACE_DIR=${3:-$DEFAULT_DERIVED_WORKSPACE_DIR} # Allow override, else use derived path

# Source the logging function
# Source the logging function
# source "$WORKSPACE_DIR/functional_tests/common/log_function.sh"

log_message() {
    local level=$1
    local msg_level=$2
    local message=$3
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [$msg_level] $message"
}

if [ -f "$PID_FILE" ]; then
    log_message "$LOG_LEVEL" "debug" "Retreiving server PID..."
    SERVER_PID=$(cat "$PID_FILE")
    log_message "$LOG_LEVEL" "notice" "Stopping server with PID $SERVER_PID..."
    kill "$SERVER_PID" 2>/dev/null
    
    # Wait for process to exit (wait command only works for child processes)
    while kill -0 "$SERVER_PID" 2>/dev/null; do
        sleep 0.1
    done
    rm -f "$PID_FILE"
    log_message "$LOG_LEVEL" "info" "Server stopped."
else
    log_message "$LOG_LEVEL" "warn" "No server PID file found. Is the server running?"
fi
