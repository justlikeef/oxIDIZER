#!/bin/bash
set -e
echo "Running Fuzzer: ox_fileproc - process_file_content"

if [ -d "ox_fileproc/fuzz" ]; then
    cd ox_fileproc
    # Run for 15 seconds max
    cargo +nightly fuzz run process_file_content -- -max_total_time=15
else
    echo "Fuzz directory not found in ox_fileproc."
    exit 1
fi
