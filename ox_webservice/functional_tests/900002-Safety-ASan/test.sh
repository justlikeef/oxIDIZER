#!/bin/bash
set -e
echo "Building ox_webservice with AddressSanitizer..."

# Source common variable if needed, but assuming standalone runnable if called from root
# Build with ASan
RUSTFLAGS="-Z sanitizer=address" cargo +nightly build -p ox_webservice --target x86_64-unknown-linux-gnu

# Find binary
SERVER_BIN="target/x86_64-unknown-linux-gnu/debug/ox_webservice"

if [ ! -f "$SERVER_BIN" ]; then
    echo "Server binary not found at $SERVER_BIN"
    exit 1
fi

echo "Starting server with ASan..."
# Run server in background
ASAN_OPTIONS="detect_odr_violation=0" $SERVER_BIN run &
SERVER_PID=$!

echo "Server PID: $SERVER_PID"
echo "Waiting for server to start..."
sleep 5

echo "Sending probe request..."
curl -v http://localhost:3000/status || true

echo "Stopping server..."
if kill -0 $SERVER_PID 2>/dev/null; then
    kill $SERVER_PID
    wait $SERVER_PID || true # Wait for it to exit
else
    echo "Server already exited."
fi

echo "ASan Test Complete."
