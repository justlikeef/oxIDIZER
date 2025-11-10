#!/bin/bash

LOG_FILE="ox_webservice.log"
SERVER_PID_FILE="ox_webservice.pid"

if [ -f "$SERVER_PID_FILE" ]; then
    SERVER_PID=$(cat $SERVER_PID_FILE)
    echo "Stopping server with PID $SERVER_PID..."
    kill $SERVER_PID 2>/dev/null
    wait $SERVER_PID 2>/dev/null
    rm -f $SERVER_PID_FILE
    echo "Server stopped."
else
    echo "No server PID file found. Is the server running?"
fi

if [ -f "$LOG_FILE" ]; then
    echo "Removing log file $LOG_FILE..."
    rm -f $LOG_FILE
    echo "Log file removed."
fi