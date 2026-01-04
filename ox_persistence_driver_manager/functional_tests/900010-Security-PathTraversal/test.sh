#!/bin/bash
set -e

# Parameters
SCRIPT_DIR=$1
TEST_LIBS_DIR=${2:-"functional_tests/common"}
MODE=$3
LOGGING_LEVEL=${4:-"info"}
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

TEST_DIR=$(dirname "$(readlink -f "$0")")
LOGS_DIR="$TEST_DIR/logs"
LOG_FILE="$LOGS_DIR/ox_persistence_security.log"

# Ensure absolute path for libs
if [[ "$TEST_LIBS_DIR" != /* ]]; then
    TEST_LIBS_DIR="$(pwd)/$TEST_LIBS_DIR"
fi

source "$TEST_LIBS_DIR/log_function.sh"

mkdir -p "$LOGS_DIR"

log_message "$LOGGING_LEVEL" "info" "Running OWASP Path Traversal Check..."
log_message "$LOGGING_LEVEL" "debug" "Output redirecting to: $LOG_FILE"

pushd ox_persistence_driver_manager > /dev/null

if cargo +nightly test --lib functional_tests_security -q > "$LOG_FILE" 2>&1; then
    log_message "$LOGGING_LEVEL" "info" "Security Check PASSED"
else
    log_message "$LOGGING_LEVEL" "error" "Security Check FAILED. Output:"
    cat "$LOG_FILE"
    exit 1
fi

popd > /dev/null
