#!/bin/bash
set -e

# ox_cc_client manual packaging script
# Requirements: cargo, git, dpkg-deb

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
PKG_ROOT="${ROOT_DIR}/crates/cc/ox_cc_client"
PKG_DIR="${ROOT_DIR}/packages"

# 1. Versioning
VERSION=$(git describe --tags --abbrev=0 2>/dev/null || echo "0.1.0")
COMMITS_SINCE_TAG=$(git rev-list "${VERSION}.." --count 2>/dev/null || echo "0")
if [ "$COMMITS_SINCE_TAG" -gt 0 ]; then
    VERSION="${VERSION}-rev${COMMITS_SINCE_TAG}"
fi
# Debian versions cannot have dashes in some places, but let's keep it simple for now
if [[ ! "$VERSION" =~ ^[0-9] ]]; then
    VERSION="0.1.0-${VERSION}"
fi

echo "--- Packaging ox_cc_client v${VERSION} ---"

# 2. Build binary
echo "Building ox_cc_client in release mode..."
cargo build --release -p ox_cc_client

# 3. Prepare staging directory
STAGING_DIR="${ROOT_DIR}/staging/deb/ox_cc_client_${VERSION}"
rm -rf "${STAGING_DIR}"
mkdir -p "${STAGING_DIR}/DEBIAN"
mkdir -p "${STAGING_DIR}/usr/bin"
mkdir -p "${STAGING_DIR}/etc/ox_cc"
mkdir -p "${STAGING_DIR}/lib/systemd/system"
mkdir -p "${STAGING_DIR}/var/lib/ox_cc"

# 4. Copy files
cp "${ROOT_DIR}/target/release/ox_cc_client" "${STAGING_DIR}/usr/bin/"
cp "${PKG_ROOT}/conf/client.yaml" "${STAGING_DIR}/etc/ox_cc/client.yaml.example"
cp "${PKG_ROOT}/installfiles/systemd/ox-cc-client.service" "${STAGING_DIR}/lib/systemd/system/"
cp "${PKG_ROOT}/installfiles/script/postinst" "${STAGING_DIR}/DEBIAN/"
chmod 755 "${STAGING_DIR}/DEBIAN/postinst"

# 5. Generate Control file
ARCH=$(dpkg --print-architecture)
MAINTAINER="oxIDIZER Team <support@oxidizer.io>"
PACKAGE_NAME="ox-cc-client"
DESCRIPTION="ox_cc secure configuration client daemon"

cat > "${STAGING_DIR}/DEBIAN/control" <<EOF
Package: ${PACKAGE_NAME}
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${ARCH}
Maintainer: ${MAINTAINER}
Depends: libc6, libssl3, ca-certificates
Description: ${DESCRIPTION}
 ox_cc is a secure configuration management client that pulls
 signed and encrypted manifests from a central server.
EOF
# Ensure final newline
echo >> "${STAGING_DIR}/DEBIAN/control"

# 6. Build the .deb
mkdir -p "${PKG_DIR}"
dpkg-deb --build "${STAGING_DIR}" "${PKG_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"

echo "------------------------------------------------"
echo "Package built successfully: ${PKG_DIR}/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
