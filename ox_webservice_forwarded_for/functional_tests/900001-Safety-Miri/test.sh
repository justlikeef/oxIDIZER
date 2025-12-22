#!/bin/bash
set -e
echo "Running Miri on ox_webservice_forwarded_for..."
cd ox_webservice_forwarded_for
cargo +nightly miri test
