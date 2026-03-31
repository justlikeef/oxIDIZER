#!/bin/bash
set -e

REPO_ROOT="/var/repos/oxIDIZER"
SCRIPTS_DIR="$REPO_ROOT/scripts"

# Build module list file
MODULE_LIST_FILE=$(mktemp /tmp/ox_test_modules_XXXXXX.txt)

find "$REPO_ROOT/crates" -name "systems_tests" -type d | sort | while read -r TEST_DIR; do
    MODULE_PATH=$(dirname "$TEST_DIR")
    echo "${MODULE_PATH#$REPO_ROOT/}"
done > "$MODULE_LIST_FILE"

"$SCRIPTS_DIR/run_systems_tests.sh" -f "$MODULE_LIST_FILE" "$@"
EXIT_CODE=$?
rm -f "$MODULE_LIST_FILE"
exit $EXIT_CODE
