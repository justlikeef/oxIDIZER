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

  # Create certs directory
  mkdir -p "$TEST_DIR/conf/certs"
  
  # Generate self-signed certificate
  openssl req -x509 -newkey rsa:4096 -keyout "$TEST_DIR/conf/certs/key.pem" -out "$TEST_DIR/conf/certs/cert.pem" -days 365 -nodes -subj "/CN=sni-test-localhost" 2>/dev/null

  # Runtime Config with heredoc
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
      # Fallback/Default
      - name: ".*"
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

  # Test SNI for localhost
  log_message "$LOGGING_LEVEL" "info" "Testing SNI for localhost..."
  SNI_CERT_SUBJECT=$(echo QUIT | openssl s_client -connect localhost:$BASE_PORT -servername localhost | openssl x509 -noout -subject)
  log_message "$LOGGING_LEVEL" "debug" "SNI Cert Subject for localhost: $SNI_CERT_SUBJECT"

  if [[ "$SNI_CERT_SUBJECT" != *"CN = sni-test-localhost"* ]]; then
    log_message "$LOGGING_LEVEL" "error" "Incorrect certificate for localhost. Expected CN=sni-test-localhost, got: $SNI_CERT_SUBJECT"
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
      cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
    fi
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
    exit $FAILED
  else
    log_message "$LOGGING_LEVEL" "notice" "Correct certificate for localhost."
  fi

  # Test fallback to default for randomhost
  log_message "$LOGGING_LEVEL" "info" "Testing fallback to default certificate for randomhost..."
  DEFAULT_CERT_SUBJECT=$(echo QUIT | openssl s_client -connect localhost:$BASE_PORT -servername randomhost | openssl x509 -noout -subject)
  log_message "$LOGGING_LEVEL" "debug" "Default Cert Subject for randomhost: $DEFAULT_CERT_SUBJECT"
  
  if [[ "$DEFAULT_CERT_SUBJECT" != *"CN = sni-test-localhost"* ]]; then
    log_message "$LOGGING_LEVEL" "error" "Incorrect default certificate for randomhost. Expected CN=sni-test-localhost, got: $DEFAULT_CERT_SUBJECT"
    if [ "$LOGGING_LEVEL" == "debug" ]; then
      log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
      cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
    fi
    "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"
    exit $FAILED
  else
    log_message "$LOGGING_LEVEL" "notice" "Correct default certificate for randomhost."
  fi

  # Stop the server
  "$SCRIPTS_DIR/stop_server.sh" "$LOGGING_LEVEL" "$TEST_PID_FILE" "$TEST_WORKSPACE_DIR"

  # Output the log file
  if [ "$LOGGING_LEVEL" == "debug" ]; then
    log_message "$LOGGING_LEVEL" "debug" "Server Logs:"
    cat "$TEST_DIR/logs/ox_webservice.log" | while read -r line; do log_message "$LOGGING_LEVEL" "debug" "  $line"; done
  fi

  log_message "$LOGGING_LEVEL" "info" "Test PASSED"
  exit $PASSED
fi

log_message "$LOGGING_LEVEL" "error" "Invalid mode: $MODE"
exit $FAILED
