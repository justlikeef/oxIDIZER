#!/bin/bash

LOG_FILE="ox_webservice.log"
SERVER_PID_FILE="ox_webservice.pid"

# Ensure previous server is stopped and log file is clean
kill $(cat $SERVER_PID_FILE 2>/dev/null) 2>/dev/null
rm -f $LOG_FILE $SERVER_PID_FILE

# Start the server in the background
export LD_LIBRARY_PATH=./target/debug

/mnt/c/Users/justl/source/repos/oxDataObject/target/debug/ox_webservice -c /mnt/c/Users/justl/source/repos/oxDataObject/ox_webservice.yaml --log-level debug > $LOG_FILE 2>&1 &
SERVER_PID=$!
echo $SERVER_PID > $SERVER_PID_FILE

echo "Server started with PID $SERVER_PID. Output redirected to $LOG_FILE"