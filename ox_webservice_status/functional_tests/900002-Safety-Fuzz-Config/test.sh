#!/bin/bash
set -e
echo "Running Fuzzer: status - config_parse"
cd ox_webservice_status
cargo +nightly fuzz run config_parse -- -max_total_time=15
