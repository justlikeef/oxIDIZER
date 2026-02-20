#!/bin/bash

# Exit codes
PASSED=0
FAILED=255
SKIPPED=77

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../functional_tests/common

SCRIPTS_DIR=$1
TEST_LIBS_DIR=${2:-$DEFAULT_TEST_LIBS_DIR}
MODE=${3:-$DEFAULT_MODE}
LOGGING_LEVEL=${4:-$DEFAULT_LOGGING_LEVEL}
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

source "$TEST_LIBS_DIR/log_function.sh"

TEST_DIR=$(dirname "$(readlink -f "$0")")

if [ "$MODE" == "integrated" ]; then
  log_message "$LOGGING_LEVEL" "info" "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"
  STAGING_DIR="/tmp/ox_test_staging_$$"

  # Dynamic Config Generation
  cp "$TEST_DIR/conf/ox_webservice.yaml" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s/port: 3000/port: $BASE_PORT/g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  sed -i "s/dependency_port: 3000/dependency_port: $BASE_PORT/g" "$TEST_DIR/conf/ox_webservice.runtime.yaml"
  # End Dynamic Config

  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "debug" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  sleep 5

  FAILURES=0

  mkdir -p "$TEST_DIR/pkg_content"
  
  # === TEST CASE: Valid YAML + Zip ===
  cp "$TEST_DIR/conf/ox_package.yaml" "$TEST_DIR/pkg_content/ox_package.yaml"
  zip -j "$TEST_DIR/valid_yaml.zip" "$TEST_DIR/pkg_content/ox_package.yaml"
  
  log_message "$LOGGING_LEVEL" "info" "Testing Valid YAML Zip Upload..."
  RESP=$(curl --connect-timeout 30 --max-time 60 -s -X POST -F "package=@$TEST_DIR/valid_yaml.zip" http://127.0.0.1:$BASE_PORT/packages/upload)
  if echo "$RESP" | grep -q '"result":"success"' && echo "$RESP" | grep -q '"extracted":true'; then
       log_message "$LOGGING_LEVEL" "info" "Valid YAML Zip passed."
  else
       log_message "$LOGGING_LEVEL" "error" "Valid YAML Zip failed: $RESP"
       FAILURES=$((FAILURES + 1))
  fi

  # Cleanup
  rm -rf "$TEST_DIR/pkg_content" "$STAGING_DIR" "$TEST_DIR"/*.zip
  
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  if [ $FAILURES -eq 0 ]; then
      log_message "$LOGGING_LEVEL" "info" "Test PASSED"
      exit $PASSED
  else
      log_message "$LOGGING_LEVEL" "error" "Test FAILED with $FAILURES failures"
      exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
