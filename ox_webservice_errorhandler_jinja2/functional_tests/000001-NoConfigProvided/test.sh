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
  # Start the server and capture the output
  START_OUTPUT=$("$SCRIPTS_DIR/start_server.sh" "$TEST_DIR/ox_webservice.yaml" "$TEST_DIR/logs/ox_webservice.log")
  echo "$START_OUTPUT"
  
  # Extract the PID from the output
  SERVER_PID=$(echo "$START_OUTPUT" | grep -oP 'Server started with PID \K\d+')

  # Allow the server to start
  sleep 2

  # Check if the process is running
  if ps -p $SERVER_PID > /dev/null; then
    echo "Server process with PID $SERVER_PID is running."
    # Stop the server
    "$SCRIPTS_DIR/stop_server.sh"

    # Check for panics in the log file
    if grep -q "panic" "$TEST_DIR/logs/ox_webservice.log"; then
        echo "Panic detected in log file."
        echo "Test FAILED"
        exit $FAILED
    else
        echo "No panics detected in log file."
    fi

    # Check for correct error message in the log file
    if grep -q "Module parameters are missing. config_file parameter is required." "$TEST_DIR/logs/ox_webservice.log"; then
        echo "Found config_file missing error in log"
    else
        echo "Did not find read error in log"
        echo "Test FAILED"
        exit $FAILED
    fi

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      echo "Server Logs:"
      cat $TEST_DIR/logs/ox_webservice.log
    fi

    echo "Test PASSED"
    exit $PASSED
  else
    echo "Server process with PID $SERVER_PID is not running."

    # Output the log file
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      echo "Server Logs:"
      cat $TEST_DIR/logs/ox_webservice.log
    fi

    echo "Test FAILED"
    exit $FAILED
  fi
fi

echo "Invalid mode: $MODE"
exit $FAILED
