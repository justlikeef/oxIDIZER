#!/bin/bash
set -e
echo "Running Miri on ox_persistence_datastore_drivermanager..."
cd ox_persistence/ox_persistence_datastore_drivermanager
cargo +nightly miri test
