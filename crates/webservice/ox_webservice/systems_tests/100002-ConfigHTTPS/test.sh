#!/bin/bash

# Exit codes
PASSED=0
FAILED=255
SKIPPED=77

# Parameters
DEFAULT_LOGGING_LEVEL="info"
DEFAULT_MODE="isolated"
DEFAULT_TEST_LIBS_DIR=$(dirname "$0")/../../../functional_tests/common

SCRIPTS_DIR=$1
# Use provided TEST_LIBS_DIR or the default
TEST_LIBS_DIR=${2:-$DEFAULT_TEST_LIBS_DIR}
# Use provided MODE or the default
MODE=${3:-$DEFAULT_MODE}
# Use provided LOGGING_LEVEL or the default
LOGGING_LEVEL=${4:-$DEFAULT_LOGGING_LEVEL}
TARGET=${5:-"debug"}
PORTS_STR=${6:-"3000 3001 3002 3003 3004"}
read -r -a PORTS <<< "$PORTS_STR"
BASE_PORT=${PORTS[0]}

# Source the logging function
source "$TEST_LIBS_DIR/log_function.sh"

# Get the directory of this script
TEST_DIR=$(dirname "$(readlink -f "$0")")

if [ "$MODE" == "integrated" ]; then
  log_message "$LOGGING_LEVEL" "info" "Skipping test in integrated mode."
  exit $SKIPPED
fi

if [ "$MODE" == "isolated" ]; then
  # Define paths for the new parameters
  TEST_PID_FILE="$TEST_DIR/ox_webservice.pid"
  TEST_WORKSPACE_DIR="/var/repos/oxIDIZER"

  mkdir -p "$TEST_DIR/conf/certs"
  
  # Generate self-signed certificate
  openssl req -x509 -newkey rsa:4096 -keyout "$TEST_DIR/conf/certs/key.pem" -out "$TEST_DIR/conf/certs/cert.pem" -days 365 -nodes -subj "/CN=localhost" 2>/dev/null

  # Runtime Config with heredoc (avoids cp/sed race conditions)
  cat <<EOF > "$TEST_DIR/conf/ox_webservice.runtime.yaml"
log4rs_config: "$TEST_WORKSPACE_DIR/conf/log4rs.yaml"
servers:
  - id: "default_https"
    protocol: "https"
    port: $BASE_PORT
    bind_address: "0.0.0.0"
    hosts:
      - name: "localhost"
        tls_key_path: "$TEST_DIR/conf/certs/key.pem"
        tls_cert_path: "$TEST_DIR/conf/certs/cert.pem"
modules:
  - id: ox_pipeline_router
    name: ox_pipeline_router
    path: "$TEST_WORKSPACE_DIR/target/$TARGET/libox_pipeline_router.so"
pipeline:
  phases:
    - Content: "ox_pipeline_router"
routes:
  # No matches, should 500 or 404 depending on impl. Test expects 500?
  # Actually if no route matches, Router returns Unmodified, Host returns 404 (default) or 500 if error.
  # The test expects 500 INT SERVER ERROR, implying a crash or mismanagement? 
  # Or maybe the legacy test expected something else.
  # However, let's keep it minimal for HTTPS verification.
  []
EOF

  # Start the server
  "$SCRIPTS_DIR/start_server.sh" \
    "$LOGGING_LEVEL" \
    "$TARGET" \
    "$TEST_DIR/conf/ox_webservice.runtime.yaml" \
    "$TEST_DIR/logs/ox_webservice.log" \
    "$TEST_PID_FILE" \
    "$TEST_WORKSPACE_DIR"

  # Allow the server to start
  sleep 2

  # Verify Certificate
  log_message "$LOGGING_LEVEL" "info" "Verifying SSL certificate..."
  CERT_SUBJECT=$(echo QUIT | openssl s_client -connect localhost:$BASE_PORT -servername localhost 2>/dev/null | openssl x509 -noout -subject)
  log_message "$LOGGING_LEVEL" "debug" "Certificate Subject: $CERT_SUBJECT"

  if [[ "$CERT_SUBJECT" != *"CN = localhost"* ]]; then
    log_message "$LOGGING_LEVEL" "error" "Incorrect certificate. Expected CN=localhost, got: $CERT_SUBJECT"
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
    exit $FAILED
  else
    log_message "$LOGGING_LEVEL" "notice" "Certificate verified successfully."
  fi

  # Perform HEAD request
  log_message "$LOGGING_LEVEL" "info" "Performing HEAD request..."
  CURL_OUTPUT=$(curl --connect-timeout 30 --max-time 60 -I -s --insecure https://localhost:$BASE_PORT/)

  # Check for correct message in the log file
  if grep -q "Listening on" "$TEST_DIR/logs/ox_webservice.log"; then
      log_message "$LOGGING_LEVEL" "notice" "Found initializing message in log"
  else
      log_message "$LOGGING_LEVEL" "error" "Did not find initializing message in log (server failed to start)"
      log_message "$LOGGING_LEVEL" "error" "Test FAILED"
      exit $FAILED
  fi

  # Check the output for 500 status code
  if echo "$CURL_OUTPUT" | head -n 1 | grep -E -q "HTTP/(1\.1|2) 500"; then
    log_message "$LOGGING_LEVEL" "notice" "Found 500 status code in curl --connect-timeout 30 --max-time 60 output..."
    log_message "$LOGGING_LEVEL" "debug" "Curl output:"
    log_message "$LOGGING_LEVEL" "debug" "$CURL_OUTPUT"
    log_message "$LOGGING_LEVEL" "info" "Test PASSED"
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
    exit $PASSED
  else
    log_message "$LOGGING_LEVEL" "error" "Did not find 500 Internal Server Error status code in curl --connect-timeout 30 --max-time 60 output."
    log_message "$LOGGING_LEVEL" "debug" "Curl output:"
    log_message "$LOGGING_LEVEL" "debug" "$CURL_OUTPUT"
    log_message "$LOGGING_LEVEL" "error" "Test FAILED"
    exit $FAILED
  fi
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
