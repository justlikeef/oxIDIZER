#!/bin/bash
set -e
echo "Running Miri on ox_webservice FFI Safety..."
cd crates/webservice/ox_webservice
# Target specific test to avoid epoll issues
cargo +nightly miri test --test ffi_safety
