#!/bin/bash

# run_miri_test <crate_name> <log_level> <logs_dir>
function run_miri_test() {
    local CRATE_NAME=$1
    local LOG_LEVEL=$2
    local LOGS_DIR=$3 # This parameter is now unused for the log file path, but kept for function signature consistency.

    local LOG_NAME=$(echo "$CRATE_NAME" | tr '/' '_')
    local LOG_FILE="$LOGS_DIR/${LOG_NAME}_miri.log" # Use LOGS_DIR from parameter
    mkdir -p "$(dirname "$LOG_FILE")"
    
    log_message "$LOG_LEVEL" "info" "Running Miri on $CRATE_NAME (this may take a while)..."
    log_message "$LOG_LEVEL" "debug" "Miri output redirecting to: $LOG_FILE"
    
    local CMD="env MIRIFLAGS='-Zmiri-disable-isolation -Zmiri-ignore-leaks' cargo +nightly -q miri test"
    
    # Check if we need to cd into crate or assuming we are already there?
    # The helper is usually sourced. The caller usually cd's.
    # But let's check caller pattern.
    # Currently caller does: `cd crate_name; cargo miri test`.
    # So we should probably assume we need to cd or run in current dir.
    # Let's assume we are in the repo root and need to cd into crate, OR let the caller handle it.
    # "fuzz_utils" reused the fuzz target name.
    # Let's make `run_miri_test` take `CRATE_DIR` and cd into it.
    
    if [ -d "$CRATE_NAME" ]; then
        pushd "$CRATE_NAME" > /dev/null
    else
        log_message "$LOG_LEVEL" "warn" "Directory $CRATE_NAME not found. Assuming we are already inside it or it's current."
    fi

    if [ "$LOG_LEVEL" == "debug" ]; then
        eval "$CMD" 2>&1 | tee "$LOG_FILE"
    else
        eval "$CMD" > "$LOG_FILE" 2>&1
    fi
    local EXIT_CODE=${PIPESTATUS[0]}

    if [ -d "$CRATE_NAME" ]; then
        popd > /dev/null
    fi

    if [ $EXIT_CODE -ne 0 ]; then
        log_message "$LOG_LEVEL" "error" "Miri failed! Logs available at $LOG_FILE"
        exit 1 # Use 1 to signal failure (harness interprets as valid failure)
    fi
    
    log_message "$LOG_LEVEL" "info" "Miri passed."
}
