#!/bin/bash
set -e
echo "Running Miri on ox_webservice_ping..."
cd ox_webservice_ping
cargo +nightly miri test
