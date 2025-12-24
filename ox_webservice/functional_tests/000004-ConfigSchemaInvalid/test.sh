#!/bin/bash
set -x

# Define variables
TEST_DIR="$(dirname "$(realpath "$0")")"
WORKSPACE_DIR="/var/repos/oxIDIZER"
START_SERVER_SCRIPT="$WORKSPACE_DIR/scripts/start_server.sh"
CONFIG_FILE="$TEST_DIR/ox_webservice.yaml"

# 1. Create a config file with VALID YAML but MISSING required fields (e.g. no 'urls', no 'servers')
cat <<EOF > "$CONFIG_FILE"
log4rs_config: "conf/log4rs.yaml"
modules: []
# Missing 'urls' and 'servers' which are required by ServerConfig
EOF

# 2. Run the server and capture output
OUTPUT_FILE="$TEST_DIR/server_output.log"
"$START_SERVER_SCRIPT" "debug" "debug" "$CONFIG_FILE" "$OUTPUT_FILE"

EXIT_CODE=$?
sleep 2

# 3. Check for specific error message containing the filename
# Since we expect it to fail loading config, exit code might be non-zero (or server might just log error and exit)
# We look for "Error deserializing configuration: In file ... missing field"

if grep -q "Error deserializing configuration" "$OUTPUT_FILE"; then
    if grep -q "000004-ConfigSchemaInvalid/ox_webservice.yaml" "$OUTPUT_FILE"; then
        echo "TEST PASSED: Filename present in deserialization error."
        exit 0
    else
        echo "TEST FAILED: Error message found but filename missing."
        cat "$OUTPUT_FILE"
        exit 255
    fi
else
    echo "TEST FAILED: Expected deserialization error not found."
    cat "$OUTPUT_FILE"
    exit 255
fi
