#!/bin/bash
set -e
echo "Running Miri on ox_webservice_status..."
cd ox_webservice_status
cargo +nightly miri test
