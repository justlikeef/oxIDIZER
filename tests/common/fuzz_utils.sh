#!/bin/bash

# run_fuzz_test <target> <log_level> <logs_dir>
function run_fuzz_test() {
    local FUZZ_TARGET=$1
    local LOG_LEVEL=$2
    local LOGS_DIR=$3

    mkdir -p "$LOGS_DIR"
    local LOG_FILE="$LOGS_DIR/${FUZZ_TARGET}_fuzz.log"
    
    log_message "$LOG_LEVEL" "info" "Running Fuzzer: $FUZZ_TARGET (this may take a while)..."
    log_message "$LOG_LEVEL" "debug" "Fuzzing output redirecting to: $LOG_FILE"
    
    local CMD="cargo +nightly -q fuzz run $FUZZ_TARGET -- -max_total_time=15"
    
    # We use a pipe to capture output for log file AND potentially display it if debug.
    # But pipe swallows exit code unless we use PIPESTATUS.
    if [ "$LOG_LEVEL" == "debug" ]; then
        # In debug, we want to see it on screen AND file.
        # But log_message handles screen printing.
        # However, fuzz output is continuous.
        # Let's just tee it.
        $CMD 2>&1 | tee "$LOG_FILE"
    else
        $CMD > "$LOG_FILE" 2>&1
    fi
    
    if [ ${PIPESTATUS[0]} -ne 0 ]; then
        log_message "$LOG_LEVEL" "error" "Fuzzing failed! Logs available at $LOG_FILE"
        # Optional: Print tail of logs?
        log_message "$LOG_LEVEL" "debug" "Tail of log:"
        tail -n 20 "$LOG_FILE" | while read -r line; do log_message "$LOG_LEVEL" "debug" "  $line"; done
        exit 255
    fi
    
    log_message "$LOG_LEVEL" "info" "Fuzzing passed."
}
