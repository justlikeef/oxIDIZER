#!/bin/bash

# Determine the workspace directory dynamically
# Get the directory of the script
THIS_SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
DEFAULT_WORKSPACE_DIR="$(dirname -- "${THIS_SCRIPT_DIR}")"
WORKSPACE_DIR=${6:-DEFAULT_WORKSPACE_DIR}
source "$WORKSPACE_DIR/functional_tests/common/log_function.sh"

# Source the logging function, assuming it's in the same directory
DEFAULT_TARGET="release"
DEFAULT_LOG_FILE="$WORKSPACE_DIR/logs/ox_webservice.log"
DEFAULT_CONFIG_FILE="$WORKSPACE_DIR/configs/ox_webservice.yaml"
DEFAULT_SERVER_PID_FILE="$WORKSPACE_DIR/ox_webservice.pid"

LOG_LEVEL=${1:-notice}
TARGET=${2:-$DEFAULT_TARGET}
CONFIG_FILE=${3:-$DEFAULT_CONFIG_FILE}
LOG_FILE=${4:-$DEFAULT_LOG_FILE}
PID_FILE=${5:-$DEFAULT_SERVER_PID_FILE}

log_message "$LOG_LEVEL" "notice" "Starting server process..."
log_message "$LOG_LEVEL" "debug" "Server Executable Target: $TARGET"
log_message "$LOG_LEVEL" "debug" "Config file: $CONFIG_FILE"
log_message "$LOG_LEVEL" "debug" "Log file: $LOG_FILE"
log_message "$LOG_LEVEL" "debug" "Log level: $LOG_LEVEL"
log_message "$LOG_LEVEL" "debug" "PID File: $PID_FILE"

# Ensure previous server is stopped and log file is clean
log_message "$LOG_LEVEL" "debug" "Checking for existing server process..."
PREV_PID=$(cat "$PID_FILE" 2>/dev/null)
if [ -n "$PREV_PID" ]; then
    log_message "$LOG_LEVEL" "debug" "Found previous PID: $PREV_PID. Attempting to kill."
    kill "$PREV_PID" 2>/dev/null
fi
rm -f "$LOG_FILE" "$PID_FILE"
log_message "$LOG_LEVEL" "debug" "Cleaned up old log and PID file."

# Start the server in the background
export LD_LIBRARY_PATH="$WORKSPACE_DIR/target/$TARGET"
log_message "$LOG_LEVEL" "debug" "LD_LIBRARY_PATH set to $LD_LIBRARY_PATH"

log_message "$LOG_LEVEL" "debug" "Executing: $WORKSPACE_DIR/target/$TARGET/ox_webservice -c \"$CONFIG_FILE\" > \"$LOG_FILE\" 2>&1 &"
$WORKSPACE_DIR/target/$TARGET/ox_webservice -c "$CONFIG_FILE" > "$LOG_FILE" 2>&1 &
SERVER_PID=$!
echo "$SERVER_PID" > "$PID_FILE"
log_message "$LOG_LEVEL" "info" "Server started with PID $SERVER_PID. Output redirected to $LOG_FILE"
log_message "$LOG_LEVEL" "debug" "Using config file: $CONFIG_FILE"