#!/bin/bash
# reproduce_upload_error.sh

# Create a dummy package file
echo "This is a test package" > /tmp/test_pkg.blob

# Upload with complex Content-Type to test parsing robustness
echo "Uploading with complex header..."
# Note: we need to use special chars carefully in bash
curl -v -X POST \
  -H "Content-Type: multipart/form-data; boundary=------------------------HQfW2JPyn971vSMlmBu7Ia; charset=utf-8" \
  --data-binary @scripts/test_pkg_body.blob \
  http://127.0.0.1:3000/packages/upload

# Note: constructing the body manually to match the hardcoded boundary in header

# Check exit code
if [ $? -eq 0 ]; then
    echo "Curl finished successfully."
else
    echo "Curl failed with exit code $?."
fi
