#!/bin/bash
set -e

REPO_ROOT="/var/repos/oxIDIZER"
SCRIPTS_DIR="$REPO_ROOT/scripts"
COMMON_TESTS_DIR="$REPO_ROOT/functional_tests/common"
FULL_LOG_FILE="$REPO_ROOT/full_suite_run.log"

echo "Starting Full Functional Test Suite Run..." > "$FULL_LOG_FILE"
START_TIME=$(date +%s)

BASE_PORT=28000
FAILURES=0
FAILED_MODULES=""

# Find all modules with functional_tests, excluding the root functional_tests dir
MODULE_PATHS=$(find "$REPO_ROOT" -maxdepth 2 -name "functional_tests" -type d | sort | grep -v "$REPO_ROOT/functional_tests$")

for TEST_DIR in $MODULE_PATHS; do
    MODULE_PATH=$(dirname "$TEST_DIR")
    MODULE_NAME=$(basename "$MODULE_PATH")
    
    echo "==================================================" | tee -a "$FULL_LOG_FILE"
    echo "Running tests for module: $MODULE_NAME" | tee -a "$FULL_LOG_FILE"
    echo "==================================================" | tee -a "$FULL_LOG_FILE"
    
    # Generate port range (5 ports)
    PORTS="$BASE_PORT $(($BASE_PORT + 1)) $(($BASE_PORT + 2)) $(($BASE_PORT + 3)) $(($BASE_PORT + 4))"
    BASE_PORT=$(($BASE_PORT + 10))
    
    MODULE_LOG="$REPO_ROOT/${MODULE_NAME}_tests.log"
    
    set +e # Don't exit on test failure
    "$SCRIPTS_DIR/run_module_tests.sh" \
        "$MODULE_NAME" \
        "$PORTS" \
        "$SCRIPTS_DIR" \
        "$COMMON_TESTS_DIR" \
        "isolated" \
        "info" \
        "debug" \
        "$MODULE_LOG"
    
    EXIT_CODE=$?
    set -e
    
    cat "$MODULE_LOG" >> "$FULL_LOG_FILE"
    
    if [ $EXIT_CODE -ne 0 ]; then
        echo "Module $MODULE_NAME FAILED (Exit Code: $EXIT_CODE)" | tee -a "$FULL_LOG_FILE"
        FAILURES=$(($FAILURES + 1))
        FAILED_MODULES="$FAILED_MODULES $MODULE_NAME"
    else
        echo "Module $MODULE_NAME PASSED" | tee -a "$FULL_LOG_FILE"
    fi
    
    echo "" | tee -a "$FULL_LOG_FILE"
done

END_TIME=$(date +%s)
DURATION=$(($END_TIME - $START_TIME))

echo "==================================================" | tee -a "$FULL_LOG_FILE"
echo "Full Suite Completed in ${DURATION}s" | tee -a "$FULL_LOG_FILE"
if [ $FAILURES -eq 0 ]; then
    echo "ALL MODULES PASSED" | tee -a "$FULL_LOG_FILE"
    exit 0
else
    echo "FAILURES DETECTED: $FAILURES" | tee -a "$FULL_LOG_FILE"
    echo "Failed Modules: $FAILED_MODULES" | tee -a "$FULL_LOG_FILE"
    exit 1
fi
