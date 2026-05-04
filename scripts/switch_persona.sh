#!/bin/bash
# Switch the running server to a different persona.
# Usage: switch_persona.sh <persona> [log_level] [target]
#
# Arguments:
#   persona    - Required. Name of the persona directory under personas/.
#   log_level  - Optional. Log level (default: notice).
#   target     - Optional. Build target: debug|release|installed (default: debug).
#
# Examples:
#   ./scripts/switch_persona.sh ca
#   ./scripts/switch_persona.sh all-services notice release

THIS_SCRIPT_DIR=$(dirname "$(readlink -f "$0")")
WORKSPACE_DIR=$(dirname "$THIS_SCRIPT_DIR")

PERSONA=${1:-}
LOG_LEVEL=${2:-notice}
TARGET=${3:-debug}
PID_FILE="$WORKSPACE_DIR/ox_webservice.pid"

if [ -z "$PERSONA" ]; then
    echo "Usage: $0 <persona> [log_level] [target]"
    echo "Available personas:"
    for d in "$WORKSPACE_DIR/personas"/*/; do
        p=$(basename "$d")
        if [ -f "$d/ox_webservice.yaml" ]; then
            echo "  $p"
        fi
    done
    exit 1
fi

CONFIG_FILE="$WORKSPACE_DIR/personas/$PERSONA/ox_webservice.yaml"
if [ ! -f "$CONFIG_FILE" ]; then
    echo "Error: No config found for persona '$PERSONA' at $CONFIG_FILE"
    exit 1
fi

echo "Switching to persona: $PERSONA"

"$THIS_SCRIPT_DIR/stop_server.sh" "$LOG_LEVEL" "$PID_FILE" "$WORKSPACE_DIR"

exec "$THIS_SCRIPT_DIR/start_server.sh" "$LOG_LEVEL" "$TARGET" "$PERSONA" \
    "$WORKSPACE_DIR/logs/ox_webservice.log" "$PID_FILE" "$WORKSPACE_DIR"
