#!/bin/bash

# Exit codes
PASSED=1
FAILED=255
SKIPPED=0

# Parameters
SCRIPTS_DIR=$1
MODE=$2

# Get the directory of this script
TEST_DIR=$(dirname "$0")

if [ "$MODE" == "integrated" ]; then
  echo "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  # Start the server
  "$SCRIPTS_DIR/start_server.sh" "$TEST_DIR/ox_webservice.yaml" "$TEST_DIR/logs/ox_webservice.log"

  # Allow the server to start
  sleep 2

  # Curl the non-existent page
  CURL_OUTPUT=$(curl -s http://localhost:3000/doesnotexist.html)

  # Stop the server
  "$SCRIPTS_DIR/stop_server.sh"

  # Check the output
  if echo "$CURL_OUTPUT" | grep -q "404 Not Found"; then
    echo "Test PASSED"
    exit $PASSED
  else
    echo "Test FAILED"
    echo "Output was: $CURL_OUTPUT"
    exit $FAILED
  fi
fi

echo "Invalid mode: $MODE"
exit $FAILED
