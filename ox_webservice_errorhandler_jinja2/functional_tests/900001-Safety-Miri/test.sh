#!/bin/bash
# Checks for Miri compatibility.
# SKIPPED because tests use cargo_metadata (subprocess) to find dynamic library, which is not supported in Miri.

exit 77
