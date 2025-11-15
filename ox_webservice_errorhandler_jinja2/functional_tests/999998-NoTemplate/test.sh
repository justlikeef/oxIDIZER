#!/bin/bash

# Exit codes
PASSED=1
FAILED=255
SKIPPED=0

# Parameters
DEFAULT_LOGGING_LEVEL="info"
SCRIPTS_DIR=$1
MODE=$2
LOGGING_LEVEL=${3:-$DEFAULT_LOGGING_LEVEL}

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

  # Check for correct  message in the log file
  if grep -q "Returning generic error" "$TEST_DIR/logs/ox_webservice.log"; then
      echo "Found generic error message in log"
  else
      echo "Did not find generic error message in log"
      echo "Test FAILED"
      exit $FAILED
  fi

  # Check the output
  if echo "$CURL_OUTPUT" | grep -q "404 Not Found"; then
    echo "Found 404 Not Found in curl output..."

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      echo "Server Logs:"
      cat $TEST_DIR/logs/ox_webservice.log

      echo "Curl Output:"
      echo "$CURL_OUTPUT"
    fi

    echo "Test PASSED"
    exit $PASSED
  else
    echo "Did not find 404 Not Found in curl output..."

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      echo "Server Logs:"
      cat $TEST_DIR/logs/ox_webservice.log
    fi

    echo "Curl Output:"
    echo "$CURL_OUTPUT"

    echo "Test FAILED"   
    exit $FAILED
  fi
fi

echo "Invalid mode: $MODE"
exit $FAILED
