#!/bin/bash
set -e
echo "Running Fuzzer: ox_webservice_stream - config_parse"

if [ -d "ox_webservice_stream/fuzz" ]; then
    cd ox_webservice_stream
    cargo +nightly fuzz run config_parse -- -max_total_time=15
else
    echo "Fuzz directory not found in ox_webservice_stream."
    exit 1
fi
