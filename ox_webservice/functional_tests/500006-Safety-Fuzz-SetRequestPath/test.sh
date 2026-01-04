#!/bin/bash
set -e
# Fuzz Target: ffi_set_request_path
TEST_DIR=$(dirname "$(readlink -f "$0")")
TEST_LIBS_DIR=$(readlink -f "${2:-functional_tests/common}")
LOGS_DIR="$TEST_DIR/logs"

source "$TEST_LIBS_DIR/log_function.sh"
source "$TEST_LIBS_DIR/fuzz_utils.sh"

cd ox_webservice
run_fuzz_test "ffi_set_request_path" "$4" "$LOGS_DIR"
