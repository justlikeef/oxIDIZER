#!/bin/bash
PASSED=0
FAILED=255
SKIPPED=77

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../../functional_tests/common

SCRIPTS_DIR=$1
TEST_LIBS_DIR=${2:-$DEFAULT_TEST_LIBS_DIR}
MODE=${3:-$DEFAULT_MODE}
LOGGING_LEVEL=${4:-$DEFAULT_LOGGING_LEVEL}

source "$TEST_LIBS_DIR/log_function.sh"
source "$TEST_LIBS_DIR/fuzz_utils.sh"

TEST_DIR=$(dirname "$(readlink -f "$0")")
LOGS_DIR="$TEST_DIR/logs"

# Verify we are in repo root or adjust path to crate
CRATE_DIR="ox_persistence_driver_manager"

if [ "$MODE" == "integrated" ]; then
  log_message "$LOGGING_LEVEL" "info" "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
    if [ -d "$CRATE_DIR/fuzz" ]; then
        cd "$CRATE_DIR"
        run_fuzz_test "config_parse" "$LOGGING_LEVEL" "$LOGS_DIR"
        exit $PASSED
    else
        log_message "$LOGGING_LEVEL" "error" "Fuzz directory not found in $CRATE_DIR."
        exit $FAILED
    fi
fi

exit $FAILED
