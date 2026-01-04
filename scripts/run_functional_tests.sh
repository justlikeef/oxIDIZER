#!/bin/bash

# Default values
SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
DEFAULT_SUPPORT_SCRIPTS_DIR="$SCRIPT_DIR/scripts"
DEFAULT_TEST_LIBS_DIR="$SCRIPT_DIR/functional_tests/common"
DEFAULT_RUNNING_MODE="isolated"
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_TARGET="debug"
DEFAULT_PORT_START=3000
DEFAULT_PORT_END=3099
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
    echo "  --port-start <port>          Start of port range (default: $DEFAULT_PORT_START)"
    echo "  --port-end <port>            End of port range (default: $DEFAULT_PORT_END)"
    echo "  --build                      Pre-build the workspace before running (default: false)"
    echo "  -h, --help                   Display this help message"
}

# --- Parse command-line options ---
SUPPORT_SCRIPTS_DIR=$DEFAULT_SUPPORT_SCRIPTS_DIR
TEST_LIBS_DIR=$DEFAULT_TEST_LIBS_DIR
RUNNING_MODE=$DEFAULT_RUNNING_MODE
LOGGING_LEVEL=$DEFAULT_LOGGING_LEVEL
TARGET=$DEFAULT_TARGET
PORT_START=$DEFAULT_PORT_START
PORT_END=$DEFAULT_PORT_END

# Use getopt to parse arguments
TEMP=$(getopt -o f:s:t:m:l:h --long file:,scripts-dir:,test-libs-dir:,mode:,log-level:,help,port-start:,port-end:,build -n 'run_functional_tests.sh' -- "$@")

if [ $? != 0 ]; then
    log_message "$LOGGING_LEVEL" "error" "Terminating..."
    exit 1
fi

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
        --port-start)
            PORT_START="$2"
            shift 2
            ;;
        --port-end)
            PORT_END="$2"
            shift 2
            ;;
        --build)
            DO_BUILD="true"
            shift
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

if [ ! -r "$MODULE_FILE" ]; then
    log_message "$LOGGING_LEVEL" "error" "Module file '$MODULE_FILE' not found or not readable."
    exit 1
fi

# Calculate Max Concurrency
RANGE_SIZE=$((PORT_END - PORT_START + 1))
SLOTS=$((RANGE_SIZE / 5))

if [ $SLOTS -lt 1 ]; then
    log_message "$LOGGING_LEVEL" "error" "Port range ($PORT_START-$PORT_END) insufficient. Need at least 5 ports."
    exit 1
fi

log_message "$LOGGING_LEVEL" "info" "Running functional tests with the following configuration:"
log_message "$LOGGING_LEVEL" "info" "Parallel Slots: $SLOTS (Range: $PORT_START-$PORT_END)"
log_message "$LOGGING_LEVEL" "info" "Running Mode: $RUNNING_MODE"

# Read modules
mapfile -t MODULES < <(grep -v '^\s*#' "$MODULE_FILE")

# Check Port Availability (Optimistic check)
# Get list of currently used ports once
USED_PORTS=$(ss -tuln | awk 'NR>1 {print $5}' | awk -F: '{print $NF}' | sort -u)

for ((i=PORT_START; i<=PORT_END; i++)); do
    # Check exact match in used ports list
    if echo "$USED_PORTS" | grep -qFx "$i"; then
        log_message "$LOGGING_LEVEL" "error" "Port $i is already in use. Cannot proceed."
        exit 1
    fi
done

# Pre-build binaries if requested
if [ "$DO_BUILD" == "true" ]; then
    log_message "$LOGGING_LEVEL" "info" "Pre-building workspace as requested..."
    if ! cargo build --workspace; then
        log_message "$LOGGING_LEVEL" "error" "Pre-build failed."
        exit 1
    fi
else
     log_message "$LOGGING_LEVEL" "info" "Skipping build step (default). Use --build to enable."
fi

# --- Execution Loop ---
declare -a PIDS
declare -a GLOBAL_ALLOCATED_PORTS
declare -A PID_PORTS_MAP # PID -> Space separated ports
declare -a MODULE_LOGS # PID -> Log file path

cleanup() {
    log_message "$LOGGING_LEVEL" "info" "Cleaning up child processes..."
    for pid in "${PIDS[@]}"; do
        kill "$pid" 2>/dev/null
    done
}
trap cleanup EXIT

process_finished_job() {
    local pid=$1
    local exit_code=$2
    local log_file=${MODULE_LOGS[$pid]}
    local ports_str=${PID_PORTS_MAP[$pid]}
    
    # Check if log file exists before catting
    if [ -f "$log_file" ]; then
        # Parse results for summary
        while read -r line; do
            if [[ "$line" == *"[RESULT]"* ]]; then
                # Extract content after [RESULT]
                content=$(echo "$line" | sed 's/.*\[RESULT\] //')
                IFS=':' read -r mod_name test_name status <<< "$content"
                result_modules+=("$mod_name")
                result_test_names+=("$test_name")
                result_statuses+=("$status")
                
                if [[ "$status" == "FAILED" ]]; then
                    highest_exit_code=255
                elif [[ "$status" == "SKIPPED" && "$highest_exit_code" -ne 255 ]]; then
                    highest_exit_code=77
                elif [[ "$status" == "PASSED" && "$highest_exit_code" -eq 1 ]]; then
                    highest_exit_code=0
                fi
            else
                echo "$line"
            fi
        done < "$log_file"
        # rm -f "$log_file"
    else
        log_message "$LOGGING_LEVEL" "error" "Log file for PID $pid not found: $log_file"
    fi

    # Release ports
    read -r -a ports_to_release <<< "$ports_str"
    # Filter GLOBAL_ALLOCATED_PORTS to remove ports_to_release
    NEW_ALLOC=()
    for p in "${GLOBAL_ALLOCATED_PORTS[@]}"; do
        keep=true
        for rel in "${ports_to_release[@]}"; do
            if [ "$p" == "$rel" ]; then
                keep=false
                break
            fi
        done
        if [ "$keep" == "true" ]; then
            NEW_ALLOC+=("$p")
        fi
    done
    GLOBAL_ALLOCATED_PORTS=("${NEW_ALLOC[@]}")
    
    unset PID_PORTS_MAP[$pid]
    unset MODULE_LOGS[$pid]
    
    # Remove from PIDS array
    NEW_PIDS=()
    for p in "${PIDS[@]}"; do
        if [ "$p" != "$pid" ]; then
            NEW_PIDS+=("$p")
        fi
    done
    PIDS=("${NEW_PIDS[@]}")
}

# Function to find N free ports
find_free_ports() {
    local count=$1
    local found_ports=()
    
    # Get list of currently used ports
    local used_ports
    used_ports=$(ss -tuln | awk '{print $5}' | awk -F: '{print $NF}' | sort -u)
    
    for ((p=PORT_START; p<=PORT_END; p++)); do
        # Check if port is in used_ports
        if echo "$used_ports" | grep -q "^$p$"; then
            continue
        fi
        
        # Check if port is in GLOBAL_ALLOCATED_PORTS
        is_allocated=false
        for alloc in "${GLOBAL_ALLOCATED_PORTS[@]}"; do
            if [ "$p" == "$alloc" ]; then
                is_allocated=true
                break
            fi
        done
        
        if [ "$is_allocated" == "true" ]; then
            continue
        fi

        found_ports+=("$p")
        
        if [ ${#found_ports[@]} -eq "$count" ]; then
            echo "${found_ports[@]}"
            return 0
        fi
    done
    
    # Not enough ports
    log_message "$LOGGING_LEVEL" "debug" "find_free_ports: No ports available in range $PORT_START-$PORT_END"
    return 1 
}

# Run Modules
MODULE_INDEX=0
for module_line in "${MODULES[@]}"; do
    # Parse module line
    read -r -a LINE_PARTS <<< "$module_line"
    MODULE_NAME="${LINE_PARTS[0]}"
    [ -z "$MODULE_NAME" ] && continue
    
    # Wait for available ports
    while true; do
        NEEDED_PORTS=5 # Default requirement
        PORTS_STR=$(find_free_ports $NEEDED_PORTS)
        
        if [ -n "$PORTS_STR" ]; then
             log_message "$LOGGING_LEVEL" "debug" "Allocated ports: $PORTS_STR for $MODULE_NAME"
             break
        fi
        
        # Wait for something to finish
        if [ ${#PIDS[@]} -gt 0 ]; then
             wait -n -p finished_pid
             exit_code=$?
             process_finished_job "$finished_pid" "$exit_code"
        else
             sleep 1
        fi
    done

    # Launch
    read -r -a P_ARRAY <<< "$PORTS_STR"
    GLOBAL_ALLOCATED_PORTS+=("${P_ARRAY[@]}")
    
    SAFE_MODULE_NAME=$(echo "$MODULE_NAME" | tr '/' '_' | tr '.' '_')
    TMP_LOG="/tmp/ox_test_${SAFE_MODULE_NAME}_${MODULE_INDEX}.log"
    ((MODULE_INDEX++))
    
    "$SUPPORT_SCRIPTS_DIR/run_module_tests.sh" "$MODULE_NAME" "$PORTS_STR" "$SUPPORT_SCRIPTS_DIR" "$TEST_LIBS_DIR" "$RUNNING_MODE" "$LOGGING_LEVEL" "$TARGET" "$TMP_LOG" &
    PID=$!
    
    log_message "$LOGGING_LEVEL" "info" "Launched $MODULE_NAME (PID: $PID) with ports: $PORTS_STR"
    
    PIDS+=("$PID")
    PID_PORTS_MAP[$PID]="$PORTS_STR"
    MODULE_LOGS[$PID]=$TMP_LOG
done

# Wait for remaining jobs
for pid in "${PIDS[@]}"; do
    if wait "$pid"; then
         process_finished_job "$pid" 0
    else
         process_finished_job "$pid" $?
    fi
done

log_message "$LOGGING_LEVEL" "info" "All modules completed."

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

# Create horizontal line segments
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
        "FAILED"*) COLOR=$RED ;;
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
