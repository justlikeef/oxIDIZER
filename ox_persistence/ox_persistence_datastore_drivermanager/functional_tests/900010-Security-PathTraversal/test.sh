#!/bin/bash
set -e
echo "Running OWASP Path Traversal Check..."
cd ox_persistence/ox_persistence_datastore_drivermanager
# We need to expose the new test file
cargo test --lib functional_tests_security
