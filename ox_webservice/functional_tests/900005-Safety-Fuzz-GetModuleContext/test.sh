#!/bin/bash
set -e
echo "Running Fuzzer: ffi_get_module_context"
cd ox_webservice
cargo +nightly fuzz run ffi_get_module_context -- -max_total_time=15
