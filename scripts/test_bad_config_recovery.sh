#!/bin/bash
set -e

CONFIG_FILE="conf/modules/active/ox_package_manager.yaml"
BACKUP_FILE="${CONFIG_FILE}.bak"

# 1. Backup existing config
if [ -f "$CONFIG_FILE" ]; then
    cp "$CONFIG_FILE" "$BACKUP_FILE"
fi

# 2. Write BAD config (missing module_id in routes)
cat > "$CONFIG_FILE" <<EOF
modules:
  - id: package_manager
    name: ox_package_manager
    params:
      config_file: /var/repos/oxIDIZER/ox_package_manager/conf/manager.yaml
routes:
  - url: "^/packages/upload"
    # Missing module_id
    phase: Content
    priority: 100
EOF

echo "Created bad configuration in $CONFIG_FILE"

# 3. Start Server
export RUST_LOG=debug
# Stop any running instance
./scripts/stop_server.sh || true

echo "Starting server with bad config..."
./target/debug/ox_webservice -c conf/ox_webservice.yaml run > server_recovery.log 2>&1 &
SERVER_PID=$!

# Wait for startup
sleep 5

# 4. Check if running
if ! kill -0 $SERVER_PID 2>/dev/null; then
    echo "FAILED: Server crashed with bad configuration!"
    cat server_recovery.log
    # Restore backup
    if [ -f "$BACKUP_FILE" ]; then mv "$BACKUP_FILE" "$CONFIG_FILE"; fi
    exit 1
fi

echo "SUCCESS: Server is running despite bad configuration."

# 5. Check logs for error message (optional verification)
if grep -q "Failed to parse route configuration" server_recovery.log; then
    echo "Confirmed: Server logged parsing error."
else
    echo "WARNING: Server did not log expected parsing error message."
fi

# 6. Restore Config
kill $SERVER_PID || true
if [ -f "$BACKUP_FILE" ]; then
    mv "$BACKUP_FILE" "$CONFIG_FILE"
    echo "Restored original configuration."
fi

exit 0
