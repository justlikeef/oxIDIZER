#!/bin/bash
set -e
echo "Running Fuzzer: ffi_set_source_ip"
cd ox_webservice
cargo +nightly fuzz run ffi_set_source_ip -- -max_total_time=15
