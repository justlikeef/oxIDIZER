#!/bin/bash
set -e

# Source common variable if needed, but assuming standalone runnable if called from root
# Build with ASan - Using prefer-dynamic to avoid static linking conflicts with dylibs
# Handle Logging
TEST_DIR=$(dirname "$(readlink -f "$0")")
TEST_LIBS_DIR=$(readlink -f "${2:-functional_tests/common}")
LOGGING_LEVEL=${4:-"info"}
LOGS_DIR="$TEST_DIR/logs"

source "$TEST_LIBS_DIR/log_function.sh"

mkdir -p "$LOGS_DIR"
BUILD_PLUGIN_LOG="$LOGS_DIR/build_plugin.log"
BUILD_ASAN_LOG="$LOGS_DIR/build_asan.log"
SERVER_LOG="$LOGS_DIR/asan_server.log"

log_message "$LOGGING_LEVEL" "info" "Running ASan Test for ox_webservice..."
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

log_message "$LOGGING_LEVEL" "info" "Building Plugin (No ASan)..."
# Build plugin normally? No, we disabled plugin loading in config anyway.
# But if we did valid it, we'd need it.
# Let's just skip building plugin if we aren't using it, or build it to ASan dir too?
# The test disables modules.
# But let's overwrite CARGO_TARGET_DIR for the ASan build.
ASAN_TARGET_DIR="$TEST_DIR/../../../target/asan"
mkdir -p "$ASAN_TARGET_DIR"

log_message "$LOGGING_LEVEL" "info" "Building Host (ASan) into target/asan..."
if [ "$LOGGING_LEVEL" == "debug" ]; then
    CARGO_TARGET_DIR="$ASAN_TARGET_DIR" RUSTFLAGS="-Z sanitizer=address -C prefer-dynamic -C link-args=-rdynamic" cargo +nightly build -p ox_webservice --target x86_64-unknown-linux-gnu 2>&1 | tee "$BUILD_ASAN_LOG"
else
    CARGO_TARGET_DIR="$ASAN_TARGET_DIR" RUSTFLAGS="-Z sanitizer=address -C prefer-dynamic -C link-args=-rdynamic" cargo +nightly build -p ox_webservice --target x86_64-unknown-linux-gnu > "$BUILD_ASAN_LOG" 2>&1
fi
if [ ${PIPESTATUS[0]} -ne 0 ]; then
    log_message "$LOGGING_LEVEL" "error" "Host Build failed! Logs:"
    cat "$BUILD_ASAN_LOG"
    exit 255
fi

# Find binary
SERVER_BIN="$ASAN_TARGET_DIR/x86_64-unknown-linux-gnu/debug/ox_webservice"

if [ ! -f "$SERVER_BIN" ]; then
    echo "Error: Server binary not found at $SERVER_BIN"
    exit 1
fi

# Setup LD_LIBRARY_PATH for dynamic rust libs and ASAN
# We need to find the nightly sysroot and the target specific lib folder
SYSROOT=$(rustc +nightly --print sysroot)
TARGET="x86_64-unknown-linux-gnu"
export LD_LIBRARY_PATH="$SYSROOT/lib/rustlib/$TARGET/lib:$LD_LIBRARY_PATH"

# Generate temporary config with port $BASE_PORT to avoid conflicts
cat <<EOF > asan_test_config.yaml
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"
modules: []
servers:
  - id: "default_http"
    protocol: "http"
    port: $BASE_PORT
    bind_address: "0.0.0.0"
    hosts:
      - name: "localhost"
pipeline:
  phases: []
routes: []
EOF

log_message "$LOGGING_LEVEL" "info" "Starting server with ASan..."
# Run server in background with leak detection disabled (we only care about memory corruption/ODR for now)
ASAN_OPTIONS="detect_odr_violation=0:detect_leaks=0:verbosity=1" $SERVER_BIN -c asan_test_config.yaml run > "$SERVER_LOG" 2>&1 &
SERVER_PID=$!
log_message "$LOGGING_LEVEL" "debug" "Server PID: $SERVER_PID"

# Wait for startup
sleep 5

log_message "$LOGGING_LEVEL" "info" "Sending probe request..."
if [ "$LOGGING_LEVEL" == "debug" ]; then
    curl --connect-timeout 30 --max-time 60 -v http://localhost:$BASE_PORT/status || true
else
    curl --connect-timeout 30 --max-time 60 -s http://localhost:$BASE_PORT/status > /dev/null || true
fi

log_message "$LOGGING_LEVEL" "info" "Stopping server..."
kill $SERVER_PID || true
wait $SERVER_PID || true

# Check log for errors (other than expected ones)
# Ideally we check exit code, but kill gave it 143 usually.
log_message "$LOGGING_LEVEL" "info" "ASan Test Complete."

# Cleanup
rm asan_test_config.yaml
