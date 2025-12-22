#!/bin/bash
set -e
echo "Running Fuzzer: module_config_parse"

if [ -d "ox_webservice_api/fuzz" ]; then
    cd ox_webservice_api
    cargo +nightly fuzz run module_config_parse -- -max_total_time=15
else
    echo "Fuzz directory not found in ox_webservice_api."
    exit 1
fi
