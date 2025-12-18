#!/bin/bash
set -e

cargo build --workspace

echo "Starting server..."
# Run in background. We pipe stdout/stderr to avoid clutter but capture if needed
cargo run -p ox_webservice -- -c conf/ox_webservice.yaml run > server.log 2>&1 &
SERVER_PID=$!

echo "Server PID: $SERVER_PID"
echo "Waiting for server to start..."
sleep 5

echo "Fetching status page..."
curl -v http://127.0.0.1:8090/status/ > status_output.html 2> curl.log

echo "Killing server..."
kill $SERVER_PID
wait $SERVER_PID 2>/dev/null || true

echo "Checking output..."
if grep -q "Configurations" status_output.html; then
    echo "PASS: Found 'Configurations' section."
else
    echo "FAIL: 'Configurations' section not found."
    exit 1
fi

if grep -q "ox_webservice_template_jinja2" status_output.html; then
    echo "PASS: Found 'ox_webservice_template_jinja2' config."
else
    echo "FAIL: 'ox_webservice_template_jinja2' config not found."
    exit 1
fi

if grep -q "content_root" status_output.html; then
    echo "PASS: Found 'content_root' key."
else
    echo "FAIL: 'content_root' key not found."
    exit 1
fi

echo "Functional verification passed!"
exit 0
