#!/bin/bash
set -e
echo "Running Miri on ox_webservice_api..."
# We are already inside ox_webservice_api due to how run_functional_tests.sh works?
# Wait, run_functional_tests.sh executes `./functional_tests/X/test.sh`. pwd is repo root.
# But "move tests to module".
# Usually run_functional_tests.sh expects specific structure.
# But per instructions "scripts under the individual modules".
# The user said "move the safety checks... into scripts under the individual modules".
# I should ensure paths are correct. PWD is typically repo root when invoked from there,
# BUT `run_functional_tests.sh` might cd into module?
# Let's check `run_functional_tests.sh` logic again.
# It does NOT cd into module. It calls `${script}`.
# So I should cd into `ox_webservice_api`.

cd ox_webservice_api
cargo +nightly miri test
