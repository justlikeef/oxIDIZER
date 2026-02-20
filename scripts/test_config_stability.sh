#!/bin/bash
set -e

# Build the server
cargo build --bin ox_webservice

# Start the server in the background
export RUST_LOG=debug
./target/debug/ox_webservice -c conf/ox_webservice.yaml run > server_test.log 2>&1 &
SERVER_PID=$!
echo "Server started with PID $SERVER_PID"

# Give it a moment to initialize
sleep 5

# Check if process is still running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "Server crashed immediately!"
    cat server_test.log
    exit 1
fi

# Make a request to the package manager route to Ensure it doesn't crash
echo "Testing /packages/upload route..."
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST http://127.0.0.1:3000/packages/upload || true)

echo "Response Code: $HTTP_CODE"

# Check if process crashed after request
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "Server crashed after request!"
    cat server_test.log
    exit 1
fi

# Cleanup
kill $SERVER_PID || true
echo "Test Passed!"
exit 0
