#!/bin/bash
set -e
# Fuzz Target: ox_fileproc - process_file_content
TEST_DIR=$(dirname "$(readlink -f "$0")")
TEST_LIBS_DIR=$(readlink -f "${2:-functional_tests/common}")
LOGS_DIR="$TEST_DIR/logs"

source "$TEST_LIBS_DIR/log_function.sh"
source "$TEST_LIBS_DIR/fuzz_utils.sh"

cd ox_fileproc
run_fuzz_test "process_file_content" "$4" "$LOGS_DIR"
