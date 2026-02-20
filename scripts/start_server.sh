#!/bin/bash

# Determine the workspace directory dynamically
# Get the directory of the script
THIS_SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
DEFAULT_WORKSPACE_DIR="$(dirname -- "${THIS_SCRIPT_DIR}")"
WORKSPACE_DIR=${6:-$DEFAULT_WORKSPACE_DIR}

DEFAULT_TARGET="debug"
DEFAULT_LOG_LEVEL="notice"
DEFAULT_LOG_FILE="$WORKSPACE_DIR/logs/ox_webservice.log"
DEFAULT_CONFIG_FILE="$WORKSPACE_DIR/conf/ox_webservice.yaml"
DEFAULT_SERVER_PID_FILE="$WORKSPACE_DIR/ox_webservice.pid"

LOG_LEVEL=${1:-$DEFAULT_LOG_LEVEL}
TARGET=${2:-$DEFAULT_TARGET}
CONFIG_FILE=${3:-$DEFAULT_CONFIG_FILE}
LOG_FILE=${4:-$DEFAULT_LOG_FILE}
PID_FILE=${5:-$DEFAULT_SERVER_PID_FILE}

# Simple log function to replace the sourced one
log_message() {
    local CURRENT_LEVEL="$1"
    local MESSAGE_LEVEL="$2"
    local MESSAGE="$3"

    declare -A LOG_LEVELS
    LOG_LEVELS[emerg]=0
    LOG_LEVELS[fatal]=0
    LOG_LEVELS[alert]=1
    LOG_LEVELS[crit]=2
    LOG_LEVELS[error]=3
    LOG_LEVELS[warn]=4
    LOG_LEVELS[notice]=5
    LOG_LEVELS[info]=6
    LOG_LEVELS[debug]=7

    local CURRENT_LEVEL_NUM=${LOG_LEVELS[$CURRENT_LEVEL]:-0}
    local MESSAGE_LEVEL_NUM=${LOG_LEVELS[$MESSAGE_LEVEL]:-0}

    if (( MESSAGE_LEVEL_NUM <= CURRENT_LEVEL_NUM )); then
        echo "[$(date '+%Y-%m-%d %H:%M:%S')] [$MESSAGE_LEVEL] $MESSAGE"
    fi
}

log_message "$LOG_LEVEL" "notice" "Starting server process..."
log_message "$LOG_LEVEL" "debug" "Server Executable Target: $TARGET"
log_message "$LOG_LEVEL" "debug" "Config file: $CONFIG_FILE"
log_message "$LOG_LEVEL" "debug" "Log file: $LOG_FILE"
log_message "$LOG_LEVEL" "debug" "Log level: $LOG_LEVEL"
log_message "$LOG_LEVEL" "debug" "PID File: $PID_FILE"

# Ensure log directory exists
mkdir -p "$(dirname "$LOG_FILE")"

# Ensure previous server is stopped and log file is clean
log_message "$LOG_LEVEL" "debug" "Checking for existing server process..."
if [ -f "$PID_FILE" ]; then
    PREV_PID=$(cat "$PID_FILE" 2>/dev/null)
    if [ -n "$PREV_PID" ]; then
        log_message "$LOG_LEVEL" "debug" "Found previous PID: $PREV_PID. Attempting to kill."
        kill "$PREV_PID" 2>/dev/null
    fi
fi

# Clean up old log and PID file
rm -f "$LOG_FILE" "$PID_FILE"

log_message "$LOG_LEVEL" "debug" "Cleaned up old log and PID file."

# Start the server in the background
export LD_LIBRARY_PATH="$WORKSPACE_DIR/target/$TARGET"
log_message "$LOG_LEVEL" "debug" "LD_LIBRARY_PATH set to $LD_LIBRARY_PATH"

SERVER_BIN="$WORKSPACE_DIR/target/$TARGET/ox_webservice"

if [ ! -f "$SERVER_BIN" ]; then
    log_message "$LOG_LEVEL" "error" "Server binary not found at $SERVER_BIN. Did you build it?"
    exit 1
fi

cd "$WORKSPACE_DIR" || exit 1
log_message "$LOG_LEVEL" "debug" "Changed directory to $WORKSPACE_DIR"

log_message "$LOG_LEVEL" "debug" "Executing: $SERVER_BIN -c \"$CONFIG_FILE\" run > \"$LOG_FILE\" 2>&1 &"
"$SERVER_BIN" -c "$CONFIG_FILE" run > "$LOG_FILE" 2>&1 &
SERVER_PID=$!
echo "$SERVER_PID" > "$PID_FILE"
log_message "$LOG_LEVEL" "info" "Server started with PID $SERVER_PID. Output redirected to $LOG_FILE"
log_message "$LOG_LEVEL" "debug" "Using config file: $CONFIG_FILE"