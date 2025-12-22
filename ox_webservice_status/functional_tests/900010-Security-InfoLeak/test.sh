#!/bin/bash
set -e
echo "Running OWASP Info Leak Check..."
cd ox_webservice_status
cargo +nightly test --test security_infoleak
