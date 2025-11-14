#!/bin/bash

WORKSPACE_DIR="/mnt/c/Users/justl/source/repos/oxIDIZER"
SERVER_PID_FILE="$WORKSPACE_DIR/ox_webservice.pid"

if [ -f "$SERVER_PID_FILE" ]; then
    echo "Retreiving server PID..."
    SERVER_PID=$(cat $SERVER_PID_FILE)
    echo "Stopping server with PID $SERVER_PID..."
    kill $SERVER_PID 2>/dev/null
    wait $SERVER_PID 2>/dev/null
    rm -f $SERVER_PID_FILE
    echo "Server stopped."
else
    echo "No server PID file found. Is the server running?"
fi
