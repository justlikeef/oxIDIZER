#!/bin/bash
set -e
echo "Running OWASP IP Spoofing Check..."
cd ox_webservice_forwarded_for
cargo +nightly test --test security_spoofing
