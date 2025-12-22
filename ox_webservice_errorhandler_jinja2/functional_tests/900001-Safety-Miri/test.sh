#!/bin/bash
set -e
echo "Running Miri on ox_webservice_errorhandler_jinja2..."
cd ox_webservice_errorhandler_jinja2
cargo +nightly miri test
