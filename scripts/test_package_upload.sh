#!/bin/bash
set -e

# Configuration
SERVER_BIN="./target/debug/ox_webservice"
CONFIG_FILE="conf/ox_webservice.yaml"
SERVER_URL="http://127.0.0.1:3000"
STAGING_DIR="/var/tmp/ox_staging"
TEST_FILE="/tmp/test_pkg.zip"
PID_FILE="/tmp/ox_test_server.pid"

# Cleanup
function cleanup {
    echo "Cleaning up..."
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if ps -p $PID > /dev/null; then
            echo "Stopping server (PID $PID)..."
            kill $PID
            wait $PID 2>/dev/null || true
        fi
        rm "$PID_FILE"
    fi
    rm -f "$TEST_FILE"
}
trap cleanup EXIT

# 1. Setup
echo "Creating test file..."
echo "dummy content" > "$TEST_FILE"
mkdir -p "$STAGING_DIR"
rm -f "$STAGING_DIR/$(basename $TEST_FILE)"

# 2. Start Server
echo "Starting server..."
# Using -- to separate arguments for cargo run if checking, but using direct binary is faster/cleaner if built
if [ ! -f "$SERVER_BIN" ]; then
    echo "Binary not found, building..."
    cargo build --bin ox_webservice
fi

RUST_LOG=debug $SERVER_BIN -c "$CONFIG_FILE" run > server_output.log 2>&1 &
echo $! > "$PID_FILE"

# Wait for server
echo "Waiting for server to be responsive..."
RETRIES=10
until curl -s "$SERVER_URL/status" > /dev/null; do
    echo "Waiting... ($RETRIES left)"
    sleep 1
    RETRIES=$((RETRIES-1))
    if [ $RETRIES -le 0 ]; then
        echo "Server failed to start!"
        cat server_output.log 2>/dev/null || true
        cat ox_webservice.log 2>/dev/null || true
        exit 1
    fi
done

# 3. Test Upload
echo "Uploading file..."
HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" -X POST -F "package=@$TEST_FILE" "$SERVER_URL/packages/upload")

if [ "$HTTP_CODE" -ne 200 ]; then
    echo "Upload failed with status $HTTP_CODE"
    cat server_output.log
    exit 1
fi
echo "Upload request successful."

# 4. Verify File
echo "Verifying file processing..."
sleep 1 # Allow small io buffer time
if [ -f "$STAGING_DIR/$(basename $TEST_FILE)" ]; then
    echo "File found in staging directory!"
else
    echo "File NOT found in staging directory."
    exit 1
fi

echo "Functional Test Passed!"
