#!/bin/bash
set -e
echo "Running Fuzzer: errorhandler_jinja2 - config_parse"
cd ox_webservice_errorhandler_jinja2
cargo +nightly fuzz run config_parse -- -max_total_time=15
