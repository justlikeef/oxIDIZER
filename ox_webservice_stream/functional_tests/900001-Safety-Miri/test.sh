#!/bin/bash
set -e
echo "Running Miri on ox_webservice_stream..."
cd ox_webservice_stream
# Run standard tests in Miri
cargo +nightly miri test
