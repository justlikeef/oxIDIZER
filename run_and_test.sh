#!/bin/bash

set -e # Exit immediately if a command exits with a non-zero status.

# Kill any existing server processes
pkill -f ox_webservice || true

export RUST_LOG="debug"

# Define server executable and config file paths
SERVER_EXEC="./target/debug/ox_webservice"
CONFIG_FILE="/mnt/c/Users/justl/source/repos/oxDataObject/ox_webservice.yaml"
LOG_FILE="ox_webservice_test.log" # This is not used in the new script, but keeping it for consistency if needed later

# Build the project first
cargo build -p ox_webservice -p ox_webservice_errorhandler_jinja2 -p ox_content

# Start the server in the background, capturing output
echo "Starting server in foreground..."
# Use a temporary file for server output
SERVER_OUTPUT_FILE=$(mktemp)
export LD_LIBRARY_PATH="$(pwd)/target/debug:$(pwd)/target/debug/deps:$LD_LIBRARY_PATH"
"$SERVER_EXEC" --config "$CONFIG_FILE" --log-level "debug" > "$SERVER_OUTPUT_FILE" 2>&1 &
SERVER_PID=$!
echo "Server started with PID $SERVER_PID. Waiting for it to initialize..."

# Give the server a moment to start
sleep 5

# Test the server with curl
echo "Testing server with curl..."
curl_output=$(curl -v http://127.0.0.1:3000/doesnotexist.html 2>&1)
echo "$curl_output"

echo "Waiting for server to process request..."
sleep 2

# Kill the server
echo "Server stopped."
kill $SERVER_PID

# Display server output
echo "Server output:"
cat "$SERVER_OUTPUT_FILE"
rm "$SERVER_OUTPUT_FILE"