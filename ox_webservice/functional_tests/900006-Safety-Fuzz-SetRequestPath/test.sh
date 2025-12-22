#!/bin/bash
set -e
echo "Running Fuzzer: ffi_set_request_path"
cd ox_webservice
cargo +nightly fuzz run ffi_set_request_path -- -max_total_time=15
