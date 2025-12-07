#!/bin/bash

# log_message <current_log_level> <message_log_level> <message>
log_message() {
    local CURRENT_LEVEL="$1"
    local MESSAGE_LEVEL="$2"
    local MESSAGE="$3"

    # Define the order of logging levels (higher index means higher severity)
    declare -A LOG_LEVELS
    LOG_LEVELS[info]=0
    LOG_LEVELS[notice]=1
    LOG_LEVELS[warn]=2
    LOG_LEVELS[error]=3
    LOG_LEVELS[crit]=4
    LOG_LEVELS[alert]=5
    LOG_LEVELS[debug]=6
    LOG_LEVELS[emerg]=7
    LOG_LEVELS[fatal]=7 # Alias for emerg


    # Convert levels to numeric values for comparison
    local CURRENT_LEVEL_NUM=${LOG_LEVELS[$CURRENT_LEVEL]:-0} # Default to trace if not found
    local MESSAGE_LEVEL_NUM=${LOG_LEVELS[$MESSAGE_LEVEL]:-0} # Default to trace if not found

    # If the message's level is equal to or higher than the current logging level, print it
    if (( MESSAGE_LEVEL_NUM <= CURRENT_LEVEL_NUM )); then
        printf "%b\n" "[${MESSAGE_LEVEL^^}] ${MESSAGE}" >&2
    fi
}
