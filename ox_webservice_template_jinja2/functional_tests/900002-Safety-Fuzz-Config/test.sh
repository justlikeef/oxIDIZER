#!/bin/bash
set -e
echo "Running Fuzzer: ox_webservice_template_jinja2 - config_parse"

if [ -d "ox_webservice_template_jinja2/fuzz" ]; then
    cd ox_webservice_template_jinja2
    cargo +nightly fuzz run config_parse -- -max_total_time=15
else
    echo "Fuzz directory not found in ox_webservice_template_jinja2."
    exit 1
fi
