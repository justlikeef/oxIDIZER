#!/bin/bash
set -x

# Define variables
TEST_DIR="$(dirname "$(realpath "$0")")"
WORKSPACE_DIR="/var/repos/oxIDIZER"
START_SERVER_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
CONFIG_FILE="$TEST_DIR/ox_webservice.yaml"
INCLUDED_FILE="$TEST_DIR/scalar.yaml"

# 1. Create a scalar YAML file
echo "just a string" > "$INCLUDED_FILE"

# 2. Create a main config file that tries to include the scalar
cat <<EOF > "$CONFIG_FILE"
log4rs_config: "conf/log4rs.yaml"
include: "$(basename "$INCLUDED_FILE")"
EOF

# 3. Run the server and capture output
OUTPUT_FILE="$TEST_DIR/server_output.log"
"$START_SERVER_SCRIPT" "debug" "debug" "$CONFIG_FILE" "$OUTPUT_FILE"

# Wait for server to potentially start and log error
sleep 2

EXIT_CODE=$?

# 4. Check for error message containing the filename
# Expected error: "Included content is not an object"
# We want to ensure it ALSO says "In file ...ox_webservice.yaml" (the parent file)

if grep -q "Included content is not an object" "$OUTPUT_FILE"; then
    if grep -q "ox_webservice.yaml" "$OUTPUT_FILE"; then
        echo "TEST PASSED: Filename present in merge error."
        exit 0
    else
        echo "TEST FAILED: Error message found but filename missing."
        cat "$OUTPUT_FILE"
        exit 255
    fi
else
    echo "TEST FAILED: Expected include error not found."
    cat "$OUTPUT_FILE"
    exit 255
fi
