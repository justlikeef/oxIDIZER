#!/bin/bash
set -e

# Parameters
SCRIPT_DIR=$1
TEST_LIBS_DIR=${2:-"functional_tests/common"}
MODE=$3
LOGGING_LEVEL=${4:-"info"}

TEST_DIR=$(dirname "$(readlink -f "$0")")
LOGS_DIR="$TEST_DIR/logs"
LOG_FILE="$LOGS_DIR/security_spoofing.log"

# Ensure absolute path for libs
if [[ "$TEST_LIBS_DIR" != /* ]]; then
    TEST_LIBS_DIR="$(pwd)/$TEST_LIBS_DIR"
fi

source "$TEST_LIBS_DIR/log_function.sh"

mkdir -p "$LOGS_DIR"

log_message "$LOGGING_LEVEL" "info" "Running OWASP IP Spoofing Check..."
log_message "$LOGGING_LEVEL" "debug" "Output redirecting to: $LOG_FILE"

pushd ox_webservice_forwarded_for > /dev/null

if cargo +nightly test --test security_spoofing -q > "$LOG_FILE" 2>&1; then
    log_message "$LOGGING_LEVEL" "info" "Security Check PASSED"
else
    log_message "$LOGGING_LEVEL" "error" "Security Check FAILED. Output:"
    cat "$LOG_FILE"
    exit 1
fi

popd > /dev/null
