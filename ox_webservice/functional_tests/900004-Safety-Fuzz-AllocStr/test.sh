#!/bin/bash
set -e
echo "Running Fuzzer: ffi_alloc_str"
cd ox_webservice
cargo +nightly fuzz run ffi_alloc_str -- -max_total_time=15
