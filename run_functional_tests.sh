#!/bin/bash

# Default values
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
DEFAULT_SUPPORT_SCRIPTS_DIR="$SCRIPT_DIR/scripts"
DEFAULT_TEST_LIBS_DIR="$SCRIPT_DIR/functional_tests/common"
DEFAULT_RUNNING_MODE="isolated"
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_TARGET="debug"
MODULE_FILE=""

# Source the logging function
source "$DEFAULT_TEST_LIBS_DIR/log_function.sh"

# --- Function to display usage ---
usage() {
    echo "Usage: $0 -f <module_file> [options]"
    echo "Options:"
    echo "  -f, --file <file>          Specify the file containing the list of modules to test (one per line)."
    echo "  -s, --scripts-dir <dir>      Specify the support scripts directory (default: $DEFAULT_SUPPORT_SCRIPTS_DIR)"
    echo "  -t, --test-libs-dir <dir>    Specify the common test libraries directory (default: $DEFAULT_TEST_LIBS_DIR)"
    echo "  -m, --mode <mode>            Specify the running mode (default: $DEFAULT_RUNNING_MODE)"
    echo "  -l, --log-level <level>      Specify the logging level (default: $DEFAULT_LOGGING_LEVEL)"
    echo "  -h, --help                   Display this help message"
}

# --- Parse command-line options ---
SUPPORT_SCRIPTS_DIR=$DEFAULT_SUPPORT_SCRIPTS_DIR
TEST_LIBS_DIR=$DEFAULT_TEST_LIBS_DIR
RUNNING_MODE=$DEFAULT_RUNNING_MODE
LOGGING_LEVEL=$DEFAULT_LOGGING_LEVEL
TARGET=$DEFAULT_TARGET

# Use getopt to parse arguments
TEMP=$(getopt -o f:s:t:m:l:h --long file:,scripts-dir:,test-libs-dir:,mode:,log-level:,help -n 'run_functional_tests.sh' -- "$@")

if [ $? != 0 ]; then
    log_message "$LOGGING_LEVEL" "error" "Terminating..."
    exit 1
fi

# Note the quotes around '$TEMP': they are essential!
eval set -- "$TEMP"

while true; do
    case "$1" in
        -f | --file)
            MODULE_FILE="$2"
            shift 2
            ;;
        -s | --scripts-dir)
            SUPPORT_SCRIPTS_DIR="$2"
            shift 2
            ;;
        -t | --test-libs-dir)
            TEST_LIBS_DIR="$2"
            shift 2
            ;;
        -m | --mode)
            RUNNING_MODE="$2"
            shift 2
            ;;
        -l | --log-level)
            LOGGING_LEVEL="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        *)
            log_message "$LOGGING_LEVEL" "error" "Internal error!"
            exit 1
            ;;
    esac
done

# Check if a module file was provided
if [ -z "$MODULE_FILE" ]; then
    log_message "$LOGGING_LEVEL" "error" "No module file specified."
    usage
    exit 1
fi

# Check if the module file exists and is readable
if [ ! -r "$MODULE_FILE" ]; then
    log_message "$LOGGING_LEVEL" "error" "Module file '$MODULE_FILE' not found or not readable."
    exit 1
fi

# Read modules from the file into an array, ignoring lines that start with #
mapfile -t MODULES < <(grep -v '^\s*#' "$MODULE_FILE")


# --- Print configuration ---
log_message "$LOGGING_LEVEL" "info" "Running functional tests with the following configuration:"
log_message "$LOGGING_LEVEL" "info" "Module File: $MODULE_FILE"
log_message "$LOGGING_LEVEL" "info" "Support Scripts Directory: $SUPPORT_SCRIPTS_DIR"
log_message "$LOGGING_LEVEL" "info" "Test Libraries Directory: $TEST_LIBS_DIR"
log_message "$LOGGING_LEVEL" "info" "Running Mode: $RUNNING_MODE"
log_message "$LOGGING_LEVEL" "info" "Logging Level: $LOGGING_LEVEL"
log_message "$LOGGING_LEVEL" "info" "Modules to test: ${MODULES[@]}"
log_message "$LOGGING_LEVEL" "info" "--------------------------------------------------"

# --- Test execution logic ---
# Arrays to store results for all modules
declare -a result_modules
declare -a result_test_names
declare -a result_statuses

# Track the highest exit code encountered
# 1 = PASSED (lowest)
# 0 = SKIPPED
# 255 = FAILED (highest)
highest_exit_code=1

for module in "${MODULES[@]}"; do
    log_message "$LOGGING_LEVEL" "info" "Processing module: $module"

    MODULE_DIR="./$module"
    FUNCTIONAL_TESTS_DIR="$MODULE_DIR/functional_tests"

    if [ ! -d "$MODULE_DIR" ]; then
        log_message "$LOGGING_LEVEL" "warn" "Module directory '$MODULE_DIR' not found. Skipping."
        log_message "$LOGGING_LEVEL" "info" "--------------------------------------------------"
        continue
    fi

    if [ ! -d "$FUNCTIONAL_TESTS_DIR" ]; then
        log_message "$LOGGING_LEVEL" "warn" "'functional_tests' directory not found in '$module'. Skipping."
        log_message "$LOGGING_LEVEL" "info" "--------------------------------------------------"
        continue
    fi

    # Find all test.sh files and sort them numerically by their parent directory
    TEST_FILES=$(find "$FUNCTIONAL_TESTS_DIR" -type f -name "test.sh" -not -path "*/functional_tests/common/*" | sort -t/ -k4,4n)

    if [ -z "$TEST_FILES" ]; then
        log_message "$LOGGING_LEVEL" "info" "No tests found in '$FUNCTIONAL_TESTS_DIR'."
        log_message "$LOGGING_LEVEL" "info" "--------------------------------------------------"
        continue
    fi

    for test_script in $TEST_FILES; do
        TEST_DIR=$(dirname "$test_script")
        TEST_NAME=$(basename "$TEST_DIR")

        log_message "$LOGGING_LEVEL" "info" "Running test: $TEST_NAME"

        # Execute the test script
        OUTPUT=$("$test_script" "$SUPPORT_SCRIPTS_DIR" "$TEST_LIBS_DIR" "$RUNNING_MODE" "$LOGGING_LEVEL" "$TARGET")
        exit_code=$?

        result_modules+=("$module")
        result_test_names+=("$TEST_NAME")

        status=""
        case $exit_code in
            1)
                status="PASSED"
                log_message "$LOGGING_LEVEL" "debug" "$OUTPUT"
                log_message "$LOGGING_LEVEL" "info" "Result: PASSED"
                ;;
            255)
                status="FAILED"
                log_message "$LOGGING_LEVEL" "debug" "$OUTPUT"
                log_message "$LOGGING_LEVEL" "error" "Result: FAILED"
                if [ "$highest_exit_code" -ne 255 ]; then
                    highest_exit_code=255
                fi
                ;;
            0)
                status="SKIPPED"
                log_message "$LOGGING_LEVEL" "debug" "$OUTPUT"
                log_message "$LOGGING_LEVEL" "warn" "Result: SKIPPED"
                if [ "$highest_exit_code" -eq 1 ]; then
                    highest_exit_code=0
                fi
                ;;
            *)
                status="UNKNOWN($exit_code)"
                log_message "$LOGGING_LEVEL" "debug" "$OUTPUT"
                log_message "$LOGGING_LEVEL" "error" "Result: UNKNOWN (Exit Code: $exit_code)"
                ;;
        esac
        result_statuses+=("$status")
    done
done

# --- Print Grand Summary Table ---
log_message "$LOGGING_LEVEL" "info" ""

# --- Calculate dynamic column widths ---
# Define Headers
MODULE_HEADER="Module"
TEST_NAME_HEADER="Test Name"
STATUS_HEADER="Status"

# Initialize with header lengths
col1_width=${#MODULE_HEADER}
col2_width=${#TEST_NAME_HEADER}
col3_width=${#STATUS_HEADER}

for i in "${!result_modules[@]}"; do
    if (( ${#result_modules[$i]} > col1_width )); then
        col1_width=${#result_modules[$i]}
    fi
    if (( ${#result_test_names[$i]} > col2_width )); then
        col2_width=${#result_test_names[$i]}
    fi
    if (( ${#result_statuses[$i]} > col3_width )); then
        col3_width=${#result_statuses[$i]}
    fi
done

# Add some padding
# The printf format string will add one space of padding on each side for content.
# So, no extra padding needed here, just the raw max length.
col1_width=${col1_width}
col2_width=${col2_width}
col3_width=${col3_width}

# ANSI color codes
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Box drawing characters
HLINE='-'
VLINE='|'
CROSS='+'

# Create horizontal line segments using the new widths
hline1=$(printf '%*s' "$((col1_width + 2))" '' | tr ' ' "$HLINE")
hline2=$(printf '%*s' "$((col2_width + 2))" '' | tr ' ' "$HLINE")
hline3=$(printf '%*s' "$((col3_width + 2))" '' | tr ' ' "$HLINE")

# Calculate total width for the summary header
total_width=$((col1_width + col2_width + col3_width + 10))
inner_width=$((total_width - 2))
summary_hline=$(printf '%*s' "$inner_width" '' | tr ' ' "$HLINE")

# Top border of summary
log_message "$LOGGING_LEVEL" "info" "$(printf '%s%s%s' "$CROSS" "$summary_hline" "$CROSS")"

# Summary text
summary_text="TESTING SUMMARY"
summary_text_len=${#summary_text}
padding_total=$((inner_width - summary_text_len))
padding_left=$((padding_total / 2))
padding_right=$((padding_total - padding_left))
log_message "$LOGGING_LEVEL" "info" "$(printf '|%*s%s%*s|' "$padding_left" "" "$summary_text" "$padding_right" "")"

# Top border of table
log_message "$LOGGING_LEVEL" "info" "$(printf '%s%s%s%s%s%s%s' "$CROSS" "$hline1" "$CROSS" "$hline2" "$CROSS" "$hline3" "$CROSS")"

# Header
log_message "$LOGGING_LEVEL" "info" "$(printf '| %-*s | %-*s | %-*s |' "$col1_width" "$MODULE_HEADER" "$col2_width" "$TEST_NAME_HEADER" "$col3_width" "$STATUS_HEADER")"

# Header separator
log_message "$LOGGING_LEVEL" "info" "$(printf '%s%s%s%s%s%s%s' "$CROSS" "$hline1" "$CROSS" "$hline2" "$CROSS" "$hline3" "$CROSS")"

# --- Print table body ---
last_module_printed=""
for i in "${!result_modules[@]}"; do
    MODULE_NAME="${result_modules[$i]}"
    TEST_NAME="${result_test_names[$i]}"
    STATUS="${result_statuses[$i]}"

    DISPLAY_MODULE_NAME=""
    if [ "$MODULE_NAME" != "$last_module_printed" ]; then
        DISPLAY_MODULE_NAME=$MODULE_NAME
        last_module_printed=$MODULE_NAME
    fi

    COLOR=$NC
    case "$STATUS" in
        "PASSED") COLOR=$GREEN ;;
        "FAILED") COLOR=$RED ;;
        "SKIPPED") COLOR=$YELLOW ;;
    esac

    # Format the row with colors, then pass the fully rendered string to log_message
    printf -v ROW_STRING "| %-*s | %-*s | %b%-*s%b |" \
        "$col1_width" "$DISPLAY_MODULE_NAME" \
        "$col2_width" "$TEST_NAME" \
        "$COLOR" "$col3_width" "$STATUS" "$NC"

    log_message "$LOGGING_LEVEL" "info" "$ROW_STRING"
done

# Bottom border
log_message "$LOGGING_LEVEL" "info" "$(printf '%s%s%s%s%s%s%s' "$CROSS" "$hline1" "$CROSS" "$hline2" "$CROSS" "$hline3" "$CROSS")"

log_message "$LOGGING_LEVEL" "info" ""
log_message "$LOGGING_LEVEL" "info" "All specified modules tested."
exit $highest_exit_code
