#!/bin/bash
set -e
echo "Running OWASP SSTI Check..."
cd ox_webservice_template_jinja2
# Run the integration test
cargo +nightly test --lib functional_tests_security
