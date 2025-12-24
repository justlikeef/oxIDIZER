#!/bin/bash
set -e

# Parameters
SCRIPT_DIR=$1
TEST_LIBS_DIR=${2:-"functional_tests/common"}
MODE=$3
LOGGING_LEVEL=${4:-"info"}

TEST_DIR=$(dirname "$(readlink -f "$0")")
LOGS_DIR="$TEST_DIR/logs"

# Ensure absolute path for libs
if [[ "$TEST_LIBS_DIR" != /* ]]; then
    TEST_LIBS_DIR="$(pwd)/$TEST_LIBS_DIR"
fi

source "$TEST_LIBS_DIR/log_function.sh"
source "$TEST_LIBS_DIR/fuzz_utils.sh"

# Enter crate directory where `fuzz` folder resides
pushd ox_webservice_api > /dev/null

run_fuzz_test "module_config_parse" "$LOGGING_LEVEL" "$LOGS_DIR"

popd > /dev/null
