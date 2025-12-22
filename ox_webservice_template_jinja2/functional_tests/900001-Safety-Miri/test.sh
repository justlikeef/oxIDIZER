#!/bin/bash
set -e
echo "Running Miri on ox_webservice_template_jinja2..."
cd ox_webservice_template_jinja2
# Run standard tests in Miri
cargo +nightly miri test
