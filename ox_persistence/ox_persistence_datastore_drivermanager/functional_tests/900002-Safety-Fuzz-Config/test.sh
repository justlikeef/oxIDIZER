#!/bin/bash
set -e
echo "Running Fuzzer: drivermanager - config_parse"
cd ox_persistence/ox_persistence_datastore_drivermanager
cargo +nightly fuzz run config_parse -- -max_total_time=15
