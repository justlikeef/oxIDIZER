#!/bin/bash

WORKSPACE_DIR="/mnt/c/Users/justl/source/repos/oxIDIZER"
DEFAULT_LOG_FILE="$WORKSPACE_DIR/logs/ox_webservice.log"
SERVER_PID_FILE="$WORKSPACE_DIR/ox_webservice.pid"
DEFAULT_CONFIG_FILE="$WORKSPACE_DIR/configs/ox_webservice.yaml"

CONFIG_FILE=${1:-$DEFAULT_CONFIG_FILE}
LOG_FILE=${2:-$DEFAULT_LOG_FILE}

# Ensure previous server is stopped and log file is clean
kill $(cat $SERVER_PID_FILE 2>/dev/null) 2>/dev/null
rm -f $LOG_FILE $SERVER_PID_FILE

# Start the server in the background
export LD_LIBRARY_PATH=$WORKSPACE_DIR/target/debug

$WORKSPACE_DIR/target/debug/ox_webservice -c "$CONFIG_FILE" > $LOG_FILE 2>&1 &
SERVER_PID=$!
echo $SERVER_PID > $SERVER_PID_FILE

echo "Server started with PID $SERVER_PID. Output redirected to $LOG_FILE"
echo "Using config file: $CONFIG_FILE"