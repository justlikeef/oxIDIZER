#!/bin/bash
# build_ca_deb.sh — Build all ox CA server Debian packages.
#
# Usage:
#   ./scripts/build_ca_deb.sh [--release]
#
# Produces (in packages/):
#   ox-webservice_<ver>_<arch>.deb            — binary + start/stop scripts
#   ox-cert-<name>_<ver>_<arch>.deb           — one per cert plugin (×14)
#   ox-webservice-<name>_<ver>_<arch>.deb     — one per webservice module (×10)
#   ox-ca-config_<ver>_<arch>.deb             — CA configuration files
#   ox-ca-server_<ver>_<arch>.deb             — meta-package with postinst
#
# Install order on the target:
#   sudo dpkg -i ox-webservice_*.deb ox-cert-*_*.deb ox-webservice-*_*.deb ox-ca-config_*.deb ox-ca-server-*.deb

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
INSTALL_DIR="$ROOT_DIR/crates/cert/installfiles"
PKG_DIR="$ROOT_DIR/packages"
STAGING_ROOT="$ROOT_DIR/staging/deb"

BUILD_PROFILE="debug"
TARGET_DIR="$ROOT_DIR/target/debug"
for arg in "$@"; do
  [[ "$arg" == "--release" ]] && BUILD_PROFILE="release" && TARGET_DIR="$ROOT_DIR/target/release"
done

# ---------------------------------------------------------------------------
# Version
# ---------------------------------------------------------------------------
VERSION=$(git -C "$ROOT_DIR" describe --tags --abbrev=0 2>/dev/null || echo "0.1.0")
COMMITS=$(git -C "$ROOT_DIR" rev-list "${VERSION}.." --count 2>/dev/null || echo "0")
[[ "$COMMITS" -gt 0 ]] && VERSION="${VERSION}-rev${COMMITS}"
[[ ! "$VERSION" =~ ^[0-9] ]] && VERSION="0.1.0-${VERSION}"
ARCH=$(dpkg --print-architecture)
MAINTAINER="oxIDIZER Team <support@oxidizer.io>"

CERT_PLUGINS=(acme admin ca_init crl health issue notify ocsp p12 ra renew revoke ssh webhook)
WS_CRATES=(ox_auth_ip ox_webservice_errorhandler_jinja2 ox_webservice_errorhandler_json ox_webservice_forwarded_for ox_webservice_ping ox_webservice_redirect ox_webservice_status ox_webservice_stream ox_webservice_template_jinja2 ox_webservice_vary_header)

echo "--- Building ox CA packages v${VERSION} (${BUILD_PROFILE}) ---"
echo

# ---------------------------------------------------------------------------
# Build all Rust targets
# ---------------------------------------------------------------------------
echo "Compiling..."
PLUGIN_ARGS=()
for p in "${CERT_PLUGINS[@]}"; do PLUGIN_ARGS+=(-p "ox_cert_${p}"); done
WS_ARGS=()
for c in "${WS_CRATES[@]}"; do WS_ARGS+=(-p "$c"); done
cargo build \
  $([ "$BUILD_PROFILE" = "release" ] && echo "--release") \
  -p ox_webservice \
  -p ox_webservice_router \
  -p ox_webservice_restore_ip \
  "${PLUGIN_ARGS[@]}" \
  "${WS_ARGS[@]}"
echo

mkdir -p "$PKG_DIR"

# ---------------------------------------------------------------------------
# Helper: write DEBIAN/control
# ---------------------------------------------------------------------------
write_control() {
  local dir="$1" pkg="$2" desc="$3" deps="${4:-libc6}"
  cat > "$dir/DEBIAN/control" <<EOF
Package: ${pkg}
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${ARCH}
Maintainer: ${MAINTAINER}
Depends: ${deps}
Description: ${desc}
EOF
  echo >> "$dir/DEBIAN/control"
}

# ---------------------------------------------------------------------------
# 1. ox-webservice — binary + management scripts
# ---------------------------------------------------------------------------
echo "Building ox-webservice..."
S="$STAGING_ROOT/ox-webservice_${VERSION}"
rm -rf "$S"
mkdir -p "$S/DEBIAN" "$S/usr/bin" "$S/usr/share/ox_webservice" "$S/usr/lib/ox_webservice"

cp "$TARGET_DIR/ox_webservice"              "$S/usr/bin/"
cp "$TARGET_DIR/libox_webservice_router.so" "$S/usr/lib/ox_webservice/"
cp "$SCRIPT_DIR/start_server.sh"            "$S/usr/share/ox_webservice/"
cp "$SCRIPT_DIR/stop_server.sh"             "$S/usr/share/ox_webservice/"
chmod 755 "$S/usr/share/ox_webservice/"*.sh

write_control "$S" "ox-webservice" \
  "ox webservice server binary and management scripts" \
  "libc6, ca-certificates"

dpkg-deb --build "$S" "$PKG_DIR/ox-webservice_${VERSION}_${ARCH}.deb"
echo "  -> ox-webservice_${VERSION}_${ARCH}.deb"

# ---------------------------------------------------------------------------
# 2. ox-cert-<name> — one package per cert plugin
# ---------------------------------------------------------------------------
for CRATE in "${CERT_PLUGINS[@]}"; do
  PKG="ox-cert-${CRATE//_/-}"
  echo "Building ${PKG}..."
  S="$STAGING_ROOT/${PKG}_${VERSION}"
  rm -rf "$S"
  mkdir -p "$S/DEBIAN" \
           "$S/usr/lib/ox_webservice" \
           "$S/usr/share/ox_webservice/modules/available" \
           "$S/usr/share/ox_webservice/modules/active"

  # Shared library
  SO="$TARGET_DIR/libox_cert_${CRATE}.so"
  [[ -f "$SO" ]] && cp "$SO" "$S/usr/lib/ox_webservice/"

  # Module registration (template — config_file points to /etc/ox_webservice/)
  cp "$INSTALL_DIR/conf/modules/available/ox_cert_${CRATE}.yaml" \
     "$S/usr/share/ox_webservice/modules/available/"
  ln -s "../available/ox_cert_${CRATE}.yaml" \
     "$S/usr/share/ox_webservice/modules/active/ox_cert_${CRATE}.yaml"

  write_control "$S" "$PKG" \
    "ox cert ${CRATE} plugin" \
    "libc6, ox-webservice (>= ${VERSION})"

  dpkg-deb --build "$S" "$PKG_DIR/${PKG}_${VERSION}_${ARCH}.deb"
  echo "  -> ${PKG}_${VERSION}_${ARCH}.deb"
done

# ---------------------------------------------------------------------------
# 2b. ox-webservice-<name> — one package per webservice module plugin
# ---------------------------------------------------------------------------
WS_CRATES_DIR="$ROOT_DIR/crates/webservice"

# Per-package extra dependencies (beyond libc6, ox-webservice)
declare -A WS_EXTRA_DEPS
# status default config activates ox_webservice_stream for the HTML page
WS_EXTRA_DEPS["ox_webservice_status"]="ox-webservice-stream (>= ${VERSION})"
# pyo3 with auto-initialize dynamically loads Python at runtime
WS_EXTRA_DEPS["ox_webservice_wsgi"]="python3"

for CRATE in "${WS_CRATES[@]}"; do
  # Strip ox_webservice_ prefix to avoid ox-webservice-ox-webservice-* duplication
  PKG="ox-webservice-${CRATE#ox_webservice_}"
  PKG="${PKG//_/-}"
  echo "Building ${PKG}..."
  S="$STAGING_ROOT/${PKG}_${VERSION}"
  rm -rf "$S"
  mkdir -p "$S/DEBIAN" \
           "$S/usr/lib/ox_webservice" \
           "$S/usr/share/ox_webservice/modules/available" \
           "$S/usr/share/ox_webservice/modules/active"

  # Shared library
  SO="$TARGET_DIR/lib${CRATE}.so"
  [[ -f "$SO" ]] && cp "$SO" "$S/usr/lib/ox_webservice/"

  # ox_webservice_forwarded_for also loads ox_webservice_restore_ip at runtime —
  # bundle its .so here since restore_ip has no independent package
  if [[ "$CRATE" == "ox_webservice_forwarded_for" ]]; then
    RESTORE_SO="$TARGET_DIR/libox_webservice_restore_ip.so"
    [[ -f "$RESTORE_SO" ]] && cp "$RESTORE_SO" "$S/usr/lib/ox_webservice/"
  fi

  # Module conf templates (replace dev paths with production equivalents)
  # Only process files that are module registration files (top-level 'modules:' key).
  # Rules files, stream configs, and other data files are excluded here and handled
  # per-crate below where needed.
  #
  # content_jinja2_default.yaml is skipped: its referenced data config has no
  # corresponding installed content in a basic deployment.
  SKIP_MODULES='content_jinja2_default.yaml'
  if [[ -d "$WS_CRATES_DIR/$CRATE/conf" ]]; then
    for YAML in "$WS_CRATES_DIR/$CRATE/conf/"*.yaml; do
      [[ -f "$YAML" ]] || continue
      grep -q '^modules:' "$YAML" || continue
      YAML_NAME="$(basename "$YAML")"
      echo "$YAML_NAME" | grep -qE "^($SKIP_MODULES)$" && continue
      sed 's|\${{OX_BASE}}/crates/webservice/'"$CRATE"'/conf/|/etc/ox_webservice/|g;
           s|\${{OX_BASE}}/personas/common/mimetypes\.yaml|/etc/ox_webservice/mimetypes.yaml|g' \
        "$YAML" > "$S/usr/share/ox_webservice/modules/available/${YAML_NAME}"
      ln -s "../available/${YAML_NAME}" \
         "$S/usr/share/ox_webservice/modules/active/${YAML_NAME}"
    done
  fi

  # ox_webservice_stream: install a single unified content root combining the
  # blue theme CSS with the layout assets (images, js, index.html).  This is
  # what content_stream_default.yaml's single ^(.*)$ catch-all serves.
  if [[ "$CRATE" == "ox_webservice_stream" ]]; then
    mkdir -p "$S/usr/share/ox_webservice/stream-content/www" \
             "$S/etc/ox_webservice"
    cp -r "$ROOT_DIR/content/layouts/default/www/." \
          "$S/usr/share/ox_webservice/stream-content/www/"
    cp -r "$ROOT_DIR/content/themes/blue/www/." \
          "$S/usr/share/ox_webservice/stream-content/www/"
    cat > "$S/etc/ox_webservice/layout.yaml" <<'EOF'
content_root: "/usr/share/ox_webservice/stream-content/www"
mimetypes_file: "/etc/ox_webservice/mimetypes.yaml"
default_documents:
  - document: "index.html"
on_content_conflict: "skip"
EOF
  fi

  # ox_webservice_status: install the status HTML content and generate a
  # stream config that points to the installed path
  if [[ "$CRATE" == "ox_webservice_status" ]]; then
    mkdir -p "$S/usr/share/ox_webservice/status" \
             "$S/etc/ox_webservice"
    cp -r "$WS_CRATES_DIR/$CRATE/content/www/status/." \
          "$S/usr/share/ox_webservice/status/"
    cp "$WS_CRATES_DIR/$CRATE/conf/status.yaml" "$S/etc/ox_webservice/"
    cat > "$S/etc/ox_webservice/ox_webservice_status_stream.yaml" <<'EOF'
content_root: "/usr/share/ox_webservice/status"
mimetypes_file: "/etc/ox_webservice/mimetypes.yaml"
default_documents:
  - document: "index.html"
on_content_conflict: "skip"
EOF
  fi

  EXTRA="${WS_EXTRA_DEPS[$CRATE]:-}"
  DEPS="libc6, ox-webservice (>= ${VERSION})"
  [[ -n "$EXTRA" ]] && DEPS="${DEPS}, ${EXTRA}"

  write_control "$S" "$PKG" \
    "ox webservice ${CRATE} module" \
    "$DEPS"

  dpkg-deb --build "$S" "$PKG_DIR/${PKG}_${VERSION}_${ARCH}.deb"
  echo "  -> ${PKG}_${VERSION}_${ARCH}.deb"
done

# ---------------------------------------------------------------------------
# 2c. ox-content-layout-default — default HTML layout and error templates
# ---------------------------------------------------------------------------
echo "Building ox-content-layout-default..."
S="$STAGING_ROOT/ox-content-layout-default_${VERSION}"
rm -rf "$S"
mkdir -p "$S/DEBIAN" \
         "$S/usr/share/ox_webservice/layouts/default/www" \
         "$S/usr/share/ox_webservice/layouts/default/error" \
         "$S/usr/share/ox_webservice/modules/available" \
         "$S/usr/share/ox_webservice/modules/active"

cp -r "$ROOT_DIR/content/layouts/default/www/"* \
      "$S/usr/share/ox_webservice/layouts/default/www/"
cp "$ROOT_DIR/content/layouts/default/error/"*.jinja2 \
   "$S/usr/share/ox_webservice/layouts/default/error/"

cat > "$S/usr/share/ox_webservice/layouts/default/layout.yaml" <<'EOF'
content_root: "/usr/share/ox_webservice/layouts/default/www"
mimetypes_file: "/etc/ox_webservice/mimetypes.yaml"
default_documents:
  - document: "index.html"
on_content_conflict: "skip"
EOF

cat > "$S/usr/share/ox_webservice/modules/available/ox_content_layout_default.yaml" <<'EOF'
modules:
  - id: "default_layout"
    name: "ox_webservice_stream"
    params:
      config_file: "/usr/share/ox_webservice/layouts/default/layout.yaml"
routes:
  - url: "^(.*)$"
    module_id: "default_layout"
    phase: Content
    priority: 999
    path_capture: true
EOF
ln -s "../available/ox_content_layout_default.yaml" \
   "$S/usr/share/ox_webservice/modules/active/ox_content_layout_default.yaml"

write_control "$S" "ox-content-layout-default" \
  "ox default HTML layout and error page templates" \
  "libc6, ox-webservice-stream (>= ${VERSION})"

dpkg-deb --build "$S" "$PKG_DIR/ox-content-layout-default_${VERSION}_${ARCH}.deb"
echo "  -> ox-content-layout-default_${VERSION}_${ARCH}.deb"

# ---------------------------------------------------------------------------
# 2d. ox-theme-blue / ox-theme-brown — CSS theme packages
# ---------------------------------------------------------------------------
for THEME in blue brown; do
  PKG="ox-theme-${THEME}"
  echo "Building ${PKG}..."
  S="$STAGING_ROOT/${PKG}_${VERSION}"
  rm -rf "$S"
  mkdir -p "$S/DEBIAN" \
           "$S/usr/share/ox_webservice/themes/${THEME}" \
           "$S/usr/share/ox_webservice/modules/available" \
           "$S/usr/share/ox_webservice/modules/active"

  cp -r "$ROOT_DIR/content/themes/${THEME}/www" \
        "$S/usr/share/ox_webservice/themes/${THEME}/"

  cat > "$S/usr/share/ox_webservice/themes/${THEME}/theme.yaml" <<EOF
content_root: "/usr/share/ox_webservice/themes/${THEME}/www"
mimetypes_file: "/etc/ox_webservice/mimetypes.yaml"
EOF

  cat > "$S/usr/share/ox_webservice/modules/available/ox_theme_${THEME}.yaml" <<EOF
modules:
  - id: "theme_${THEME}"
    name: "ox_webservice_stream"
    params:
      config_file: "/usr/share/ox_webservice/themes/${THEME}/theme.yaml"
routes:
  - url: "^(/css/[^?]*)$"
    module_id: "theme_${THEME}"
    phase: Content
    priority: 998
    path_capture: true
  - url: "^(/js/[^?]*)$"
    module_id: "theme_${THEME}"
    phase: Content
    priority: 998
    path_capture: true
EOF
  ln -s "../available/ox_theme_${THEME}.yaml" \
     "$S/usr/share/ox_webservice/modules/active/ox_theme_${THEME}.yaml"

  write_control "$S" "$PKG" \
    "ox ${THEME} UI theme" \
    "libc6, ox-webservice-stream (>= ${VERSION})"

  dpkg-deb --build "$S" "$PKG_DIR/${PKG}_${VERSION}_${ARCH}.deb"
  echo "  -> ${PKG}_${VERSION}_${ARCH}.deb"
done

# ---------------------------------------------------------------------------
# 2e. ox-template-jinja2 — Jinja2 layout includes
# ---------------------------------------------------------------------------
echo "Building ox-template-jinja2..."
S="$STAGING_ROOT/ox-template-jinja2_${VERSION}"
rm -rf "$S"
mkdir -p "$S/DEBIAN" \
         "$S/usr/share/ox_webservice/layouts/jinja2/includes"

cp "$ROOT_DIR/content/layouts/jijna2/includes/"*.jinja2 \
   "$S/usr/share/ox_webservice/layouts/jinja2/includes/"

write_control "$S" "ox-template-jinja2" \
  "ox Jinja2 layout template includes" \
  "libc6, ox-webservice-errorhandler-jinja2 (>= ${VERSION})"

dpkg-deb --build "$S" "$PKG_DIR/ox-template-jinja2_${VERSION}_${ARCH}.deb"
echo "  -> ox-template-jinja2_${VERSION}_${ARCH}.deb"

# ---------------------------------------------------------------------------
# 3. ox-ca-config — all CA configuration files
# ---------------------------------------------------------------------------
echo "Building ox-ca-config..."
S="$STAGING_ROOT/ox-ca-config_${VERSION}"
rm -rf "$S"
mkdir -p "$S/DEBIAN" \
         "$S/etc/ox_webservice/service" \
         "$S/etc/ox_webservice/servers" \
         "$S/etc/ox_webservice/modules/available" \
         "$S/etc/ox_webservice/modules/active" \
         "$S/usr/share/ox_webservice"

# Webservice entry point and structure configs
cp "$INSTALL_DIR/conf/ox_webservice.yaml"   "$S/etc/ox_webservice/"
cp "$INSTALL_DIR/conf/service/base.yaml"    "$S/etc/ox_webservice/service/"
cp "$INSTALL_DIR/conf/servers/servers.yaml" "$S/etc/ox_webservice/servers/"

# Module configs: copy to available/, symlink from active/
cp "$INSTALL_DIR/conf/modules/available/"*.yaml "$S/etc/ox_webservice/modules/available/"
for f in "$S/etc/ox_webservice/modules/available/"*.yaml; do
  ln -s "../available/$(basename "$f")" \
        "$S/etc/ox_webservice/modules/active/$(basename "$f")"
done

# Plugin configs — replace localhost placeholder
for CRATE in "${CERT_PLUGINS[@]}"; do
  src="$ROOT_DIR/crates/cert/ox_cert_${CRATE}/conf/plugin.yaml"
  [[ -f "$src" ]] && \
    sed 's|http://localhost:8080|http://YOUR_CA_HOSTNAME:8080|g' \
      "$src" > "$S/etc/ox_webservice/${CRATE}.yaml"
done

# HTTPS redirect rules config — hostname placeholder replaced by postinst
# ACME http-01 challenge requests must be served over plain HTTP on port 80
cat > "$S/etc/ox_webservice/https_redirect.yaml" <<'EOF'
rules:
  - match_pattern: "^/.well-known/acme-challenge/"
    skip: true
  - match_pattern: "^(.*)$"
    replace_string: "https://YOUR_CA_HOSTNAME$1"
EOF

# CA root redirect — sends bare / to /ca/
cat > "$S/etc/ox_webservice/ca_root_redirect.yaml" <<'EOF'
rules:
  - match_pattern: "^/$"
    replace_string: "/ca/"
EOF

# Stream plugin config (points to installed content)
cat > "$S/etc/ox_webservice/stream.yaml" <<'EOF'
content_root: "/usr/share/ox_webservice/content/www"
mimetypes_file: "/etc/ox_webservice/mimetypes.yaml"
default_documents:
  - document: "index.html"
on_content_conflict: "skip"
EOF

# Mimetypes and logging
cp "$ROOT_DIR/personas/common/mimetypes.yaml" "$S/etc/ox_webservice/"
cat > "$S/etc/ox_webservice/log4rs.yaml" <<'EOF'
refresh_rate: 30 seconds
appenders:
  file:
    kind: file
    path: "/var/log/ox_webservice/ox_webservice.log"
    encoder:
      pattern: "{d} {l} {t} - {m}{n}"
root:
  level: info
  appenders:
    - file
EOF

# Jinja2 error handler config (error templates live in ox-content-layout-default)
cat > "$S/etc/ox_webservice/errorhandler_jinja2.yaml" <<'EOF'
content_root: "/usr/share/ox_webservice/layouts/default/error"
EOF

# CA trust page content and operational scripts
cp -r "$ROOT_DIR/crates/cert/ox_cert_admin/content" "$S/usr/share/ox_webservice/"
cp "$SCRIPT_DIR/generate-root-ca.sh"              "$S/usr/share/ox_webservice/"
cp "$SCRIPT_DIR/install-ca-cert.sh"               "$S/usr/share/ox_webservice/"
cp "$INSTALL_DIR/script/check-ca-key.sh"          "$S/usr/share/ox_webservice/"
cp "$INSTALL_DIR/script/unlock-ca.sh"             "$S/usr/share/ox_webservice/"
chmod 755 "$S/usr/share/ox_webservice/"*.sh

# conffiles — preserve admin edits on upgrade
find "$S/etc/ox_webservice" -type f | sed "s|$S||" | sort > "$S/DEBIAN/conffiles"

# Build dependency list: all cert plugin packages + required webservice/content packages
PLUGIN_DEPS=$(printf "ox-cert-%s (>= ${VERSION}), " "${CERT_PLUGINS[@]//_/-}")
PLUGIN_DEPS="${PLUGIN_DEPS%, }"
CA_WS_DEPS="ox-webservice-redirect (>= ${VERSION}), ox-webservice-stream (>= ${VERSION}), ox-webservice-errorhandler-jinja2 (>= ${VERSION}), ox-webservice-errorhandler-json (>= ${VERSION})"
CA_CONTENT_DEPS="ox-content-layout-default (>= ${VERSION}), ox-theme-blue (>= ${VERSION})"

write_control "$S" "ox-ca-config" \
  "ox Certificate Authority configuration files" \
  "ox-webservice (>= ${VERSION}), ${PLUGIN_DEPS}, ${CA_WS_DEPS}, ${CA_CONTENT_DEPS}, openssl"

dpkg-deb --build "$S" "$PKG_DIR/ox-ca-config_${VERSION}_${ARCH}.deb"
echo "  -> ox-ca-config_${VERSION}_${ARCH}.deb"

# ---------------------------------------------------------------------------
# 4. ox_ca_server — meta-package with postinst and systemd unit
# ---------------------------------------------------------------------------
echo "Building ox_ca_server..."
S="$STAGING_ROOT/ox_ca_server_${VERSION}"
rm -rf "$S"
mkdir -p "$S/DEBIAN" \
         "$S/lib/systemd/system" \
         "$S/var/lib/ox_webservice" \
         "$S/var/log/ox_webservice" \
         "$S/var/run/ox_webservice"

cp "$INSTALL_DIR/systemd/ox_webservice.service" "$S/lib/systemd/system/"
cp "$INSTALL_DIR/script/postinst" "$S/DEBIAN/"
chmod 755 "$S/DEBIAN/postinst"

write_control "$S" "ox-ca-server" \
  "ox Certificate Authority Server (meta-package)" \
  "ox-ca-config (>= ${VERSION})"

dpkg-deb --build "$S" "$PKG_DIR/ox-ca-server-${VERSION}_${ARCH}.deb"
echo "  -> ox-ca-server-${VERSION}_${ARCH}.deb"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
echo "Packages written to: $PKG_DIR"
echo
echo "Install on target (in one dpkg call — resolves order automatically):"
echo "  scp $PKG_DIR/*_${VERSION}_${ARCH}.deb user@ca-server:"
echo "  ssh user@ca-server 'sudo dpkg -i \\"
echo "    ox-webservice_${VERSION}_${ARCH}.deb \\"
for CRATE in "${CERT_PLUGINS[@]}"; do
  echo "    ox-cert-${CRATE//_/-}_${VERSION}_${ARCH}.deb \\"
done
for CRATE in "${WS_CRATES[@]}"; do
  echo "    ox-webservice-${CRATE//_/-}_${VERSION}_${ARCH}.deb \\"
done
echo "    ox-ca-config_${VERSION}_${ARCH}.deb \\"
echo "    ox-ca-server_${VERSION}_${ARCH}.deb'"
