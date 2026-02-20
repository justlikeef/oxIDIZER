#!/bin/bash

# run_module_tests.sh
# Runs all functional tests for a specific module sequentially, using a designated port block.

# Parameters
MODULE_NAME=$1
PORTS_STR=$2 # Space separated list of ports
SUPPORT_SCRIPTS_DIR=$3
TEST_LIBS_DIR=$4
RUNNING_MODE=$5
LOGGING_LEVEL=$6
TARGET=$7
RESULT_LOG=$8  # Where to write the unified output for this module
SPECIFIC_TESTS=$9 # Optional: Space separated list of specific tests to run (e.g. "000001 000002")

# Validate critical args
echo "DEBUG_ARGS: 1='$1' 2='$2' 3='$3' 4='$4' 5='$5' 6='$6' 7='$7' 8='$8' 9='$9'"
if [ -z "$MODULE_NAME" ] || [ -z "$PORTS_STR" ] || [ -z "$RESULT_LOG" ]; then
    echo "Error: Missing arguments for run_module_tests.sh"
    exit 1
fi

# Initialize log file
> "$RESULT_LOG"

# Source logging (we won't use log_message directly for console output to avoid interleaving,
# instead we append to RESULT_LOG)
source "$TEST_LIBS_DIR/log_function.sh"

log_to_file() {
    local level=$1
    local msg=$2
    # Simple formatting matching log_function.sh style if possible, or just raw
    echo "[$level] $msg" >> "$RESULT_LOG"
}

log_to_file "INFO" "=================================================="
log_to_file "INFO" "STARTING MODULE: $MODULE_NAME (Ports: $PORTS_STR)"
if [ -n "$SPECIFIC_TESTS" ]; then
    log_to_file "INFO" "RUNNING SPECIFIC TESTS: $SPECIFIC_TESTS"
fi
log_to_file "INFO" "=================================================="

MODULE_DIR="./$MODULE_NAME"
FUNCTIONAL_TESTS_DIR="$MODULE_DIR/functional_tests"

if [ ! -d "$MODULE_DIR" ]; then
    log_to_file "WARN" "Module directory '$MODULE_DIR' not found. Skipping."
    exit 0
fi

if [ ! -d "$FUNCTIONAL_TESTS_DIR" ]; then
    log_to_file "WARN" "'functional_tests' directory not found. Skipping."
    exit 0
fi

if [ -n "$SPECIFIC_TESTS" ]; then
    # Filter for specific tests
    TEST_FILES=""
    for test_id in $SPECIFIC_TESTS; do
        # Try exact match, then try prefix match
        MATCH=$(find "$FUNCTIONAL_TESTS_DIR" -maxdepth 1 -type d -name "$test_id*" | head -n 1)
        
        if [ -n "$MATCH" ] && [ -f "$MATCH/test.sh" ]; then
            TEST_FILES="$TEST_FILES $MATCH/test.sh"
        else
            log_to_file "WARN" "Specific test matching '$test_id' not found in $FUNCTIONAL_TESTS_DIR."
        fi
    done
else
    # Find all test.sh files
    TEST_FILES=$(find "$FUNCTIONAL_TESTS_DIR" -type f -name "test.sh" -not -path "*/functional_tests/common/*" | sort)
fi

if [ -z "$TEST_FILES" ]; then
    log_to_file "INFO" "No tests found."
    exit 0
fi

MODULE_EXIT_CODE=0
FAILURES=0
TOTAL_TESTS=0

for test_script in $TEST_FILES; do
    TEST_DIR=$(dirname "$test_script")
    TEST_NAME=$(basename "$TEST_DIR")
    
    log_to_file "INFO" "Running test: $TEST_NAME"
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    
    # Execute Test Script
    # Pass PORTS_STR as the 7th argument (shifting logs/target)
    # Args: SCRIPTS_DIR LIBS_DIR MODE LOG_LEVEL TARGET PORTS_STR
    OUTPUT=$("$test_script" "$SUPPORT_SCRIPTS_DIR" "$TEST_LIBS_DIR" "$RUNNING_MODE" "$LOGGING_LEVEL" "$TARGET" "$PORTS_STR" 2>&1)
    EXIT_CODE=$?
    
    STATUS=""
    if [ $EXIT_CODE -eq 0 ]; then
         STATUS="PASSED"
         log_to_file "INFO" "Result: PASSED"
         log_to_file "RESULT" "$MODULE_NAME:$TEST_NAME:PASSED"
    elif [ $EXIT_CODE -eq 77 ]; then
         STATUS="SKIPPED"
         log_to_file "WARN" "Result: SKIPPED"
         log_to_file "RESULT" "$MODULE_NAME:$TEST_NAME:SKIPPED"
    else
         STATUS="FAILED"
         log_to_file "ERROR" "$OUTPUT"
         log_to_file "ERROR" "Result: FAILED (Exit: $EXIT_CODE)"
         log_to_file "RESULT" "$MODULE_NAME:$TEST_NAME:FAILED"
         FAILURES=$((FAILURES + 1))
         MODULE_EXIT_CODE=1
    fi

    # Cleanup any processes holding the ports to avoid interference
    for port in $PORTS_STR; do
        if fuser "$port/tcp" >/dev/null 2>&1; then
             # log_to_file "WARN" "Cleaning up stale process on port $port"
             fuser -k -n tcp "$port" >/dev/null 2>&1
        fi
    done
done

log_to_file "INFO" "Module $MODULE_NAME completed. Tests: $TOTAL_TESTS, Failures: $FAILURES"
log_to_file "INFO" "--------------------------------------------------"

if [ $MODULE_EXIT_CODE -ne 0 ]; then
    exit 255
else
    exit 0
fi
