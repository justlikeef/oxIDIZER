#!/bin/bash
set -e
echo "Running Fuzzer: ffi_set_request_header"
cd ox_webservice
cargo +nightly fuzz run ffi_set_request_header -- -max_total_time=15
