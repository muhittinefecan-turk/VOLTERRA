#!/usr/bin/env bash
# ==============================================================================
# VOLTERRA Universal macOS Binary Build Pipeline (arm64 + x86_64)
# Production Build & Packaging Script for crates/volterra-ffi -> apps/VolterraMac
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

TARGET_ARM64="aarch64-apple-darwin"
TARGET_X86_64="x86_64-apple-darwin"

BUILD_MODE="${1:-release}"
PROFILE_FLAG="--release"
TARGET_SUBDIR="release"

if [ "$BUILD_MODE" = "debug" ]; then
    PROFILE_FLAG=""
    TARGET_SUBDIR="debug"
fi

OUTPUT_DIR="${WORKSPACE_ROOT}/target/universal/${TARGET_SUBDIR}"
BRIDGE_DIR="${WORKSPACE_ROOT}/apps/VolterraMac/Sources/Bridge"

echo "========================================================================"
echo "VOLTERRA Universal Binary Build Pipeline"
echo "Mode: ${BUILD_MODE} (${TARGET_SUBDIR})"
echo "Output Directory: ${OUTPUT_DIR}"
echo "Bridge Directory: ${BRIDGE_DIR}"
echo "========================================================================"

# Step 1: Ensure Target Toolchains are Installed
echo "==> Step 1: Verifying Rust target toolchains..."
if command -v rustup >/dev/null 2>&1; then
    INSTALLED_TARGETS=$(rustup target list --installed)
    if ! echo "${INSTALLED_TARGETS}" | grep -q "${TARGET_ARM64}"; then
        echo "Installing target ${TARGET_ARM64}..."
        rustup target add "${TARGET_ARM64}"
    fi
    if ! echo "${INSTALLED_TARGETS}" | grep -q "${TARGET_X86_64}"; then
        echo "Installing target ${TARGET_X86_64}..."
        rustup target add "${TARGET_X86_64}"
    fi
else
    echo "Notice: rustup not found in PATH; assuming targets are available."
fi

# Step 2: Compile for Apple Silicon (arm64)
echo "==> Step 2: Building volterra-ffi for Apple Silicon (${TARGET_ARM64})..."
cargo build ${PROFILE_FLAG} --target "${TARGET_ARM64}" -p volterra-ffi

# Step 3: Compile for Intel Mac (x86_64)
echo "==> Step 3: Building volterra-ffi for Intel (${TARGET_X86_64})..."
cargo build ${PROFILE_FLAG} --target "${TARGET_X86_64}" -p volterra-ffi

# Step 4: Create Universal Fat Binaries via lipo
echo "==> Step 4: Packaging Universal Fat Libraries via lipo..."
mkdir -p "${OUTPUT_DIR}"
mkdir -p "${BRIDGE_DIR}"

STATIC_ARM="${WORKSPACE_ROOT}/target/${TARGET_ARM64}/${TARGET_SUBDIR}/libvolterra_ffi.a"
STATIC_X86="${WORKSPACE_ROOT}/target/${TARGET_X86_64}/${TARGET_SUBDIR}/libvolterra_ffi.a"
STATIC_UNI="${OUTPUT_DIR}/libvolterra_ffi.a"

DYLIB_ARM="${WORKSPACE_ROOT}/target/${TARGET_ARM64}/${TARGET_SUBDIR}/libvolterra_ffi.dylib"
DYLIB_X86="${WORKSPACE_ROOT}/target/${TARGET_X86_64}/${TARGET_SUBDIR}/libvolterra_ffi.dylib"
DYLIB_UNI="${OUTPUT_DIR}/libvolterra_ffi.dylib"

# 4a. Merge Static Archives
if [ -f "${STATIC_ARM}" ] && [ -f "${STATIC_X86}" ]; then
    echo "Creating Universal static library: ${STATIC_UNI}"
    /usr/bin/lipo -create "${STATIC_ARM}" "${STATIC_X86}" -output "${STATIC_UNI}"
    cp "${STATIC_UNI}" "${BRIDGE_DIR}/libvolterra_ffi.a"
else
    echo "Error: Static archive slice missing! arm64: ${STATIC_ARM}, x86_64: ${STATIC_X86}" >&2
    exit 1
fi

# 4b. Merge Dynamic Libraries and Configure Install Name / rpath
if [ -f "${DYLIB_ARM}" ] && [ -f "${DYLIB_X86}" ]; then
    echo "Creating Universal dynamic library: ${DYLIB_UNI}"
    /usr/bin/lipo -create "${DYLIB_ARM}" "${DYLIB_X86}" -output "${DYLIB_UNI}"
    
    echo "Configuring LC_ID_DYLIB with install_name_tool..."
    /usr/bin/install_name_tool -id "@rpath/libvolterra_ffi.dylib" "${DYLIB_UNI}"
    
    # Ad-hoc sign on macOS to prevent codesign verification failure on Apple Silicon
    if command -v codesign >/dev/null 2>&1; then
        echo "Ad-hoc code signing universal dylib..."
        /usr/bin/codesign --force --sign - "${DYLIB_UNI}"
    fi
    
    cp "${DYLIB_UNI}" "${BRIDGE_DIR}/libvolterra_ffi.dylib"
else
    echo "Error: Dynamic library slice missing! arm64: ${DYLIB_ARM}, x86_64: ${DYLIB_X86}" >&2
    exit 1
fi

# Step 5: Verify Architecture Slices
echo "==> Step 5: Verifying Universal Binary Architecture Slices..."
if [ -f "${STATIC_UNI}" ]; then
    echo -n "Static Archive: "
    /usr/bin/lipo -info "${STATIC_UNI}"
fi
if [ -f "${DYLIB_UNI}" ]; then
    echo -n "Dynamic Library: "
    /usr/bin/lipo -info "${DYLIB_UNI}"
    echo "Dylib ID: $(otool -D "${DYLIB_UNI}" | tail -n 1)"
fi

# Step 6: Generate C Header via cbindgen
echo "==> Step 6: Generating / Updating C Header (volterra_ffi.h)..."
CBINDGEN_CONFIG="${WORKSPACE_ROOT}/crates/volterra-ffi/cbindgen.toml"
CBINDGEN_OUTPUT="${BRIDGE_DIR}/volterra_ffi.h"

CBINDGEN_BIN="${HOME}/.cargo/bin/cbindgen"
if [ -x "${CBINDGEN_BIN}" ]; then
    echo "Running ~/.cargo/bin/cbindgen..."
    "${CBINDGEN_BIN}" --config "${CBINDGEN_CONFIG}" "${WORKSPACE_ROOT}/crates/volterra-ffi" -o "${CBINDGEN_OUTPUT}"
elif command -v cbindgen >/dev/null 2>&1; then
    echo "Running cbindgen CLI..."
    cbindgen --config "${CBINDGEN_CONFIG}" "${WORKSPACE_ROOT}/crates/volterra-ffi" -o "${CBINDGEN_OUTPUT}"
elif cargo --list | grep -q cbindgen; then
    echo "Running cargo cbindgen..."
    cargo cbindgen --config "${CBINDGEN_CONFIG}" --crate volterra-ffi -o "${CBINDGEN_OUTPUT}"
else
    echo "Notice: cbindgen binary not found; running test_header_generation..."
    cargo test -p volterra-ffi --lib test_header_generation
fi

# Step 7: Build Universal SwiftUI Application & Package VOLTERRA.app
echo "==> Step 7: Building Universal SwiftUI Application (arm64 + x86_64)..."
SWIFT_CONFIG="release"
if [ "$BUILD_MODE" = "debug" ]; then
    SWIFT_CONFIG="debug"
fi

pushd "${WORKSPACE_ROOT}/apps/VolterraMac" >/dev/null
swift build -c "${SWIFT_CONFIG}" --arch arm64 --arch x86_64
SWIFT_BIN_DIR="$(swift build -c "${SWIFT_CONFIG}" --arch arm64 --arch x86_64 --show-bin-path)"
popd >/dev/null

APP_BUNDLE="${OUTPUT_DIR}/VOLTERRA.app"
rm -rf "${APP_BUNDLE}"
mkdir -p "${APP_BUNDLE}/Contents/MacOS"
mkdir -p "${APP_BUNDLE}/Contents/Frameworks"
mkdir -p "${APP_BUNDLE}/Contents/Resources"

cp "${SWIFT_BIN_DIR}/VolterraMac" "${APP_BUNDLE}/Contents/MacOS/VolterraMac"
cp "${DYLIB_UNI}" "${APP_BUNDLE}/Contents/Frameworks/libvolterra_ffi.dylib"
/usr/bin/install_name_tool -add_rpath "@executable_path/../Frameworks" "${APP_BUNDLE}/Contents/MacOS/VolterraMac" 2>/dev/null || true

cat > "${APP_BUNDLE}/Contents/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>VolterraMac</string>
    <key>CFBundleIdentifier</key>
    <string>com.volterra.mac</string>
    <key>CFBundleName</key>
    <string>VOLTERRA</string>
    <key>CFBundleDisplayName</key>
    <string>VOLTERRA</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

if command -v codesign >/dev/null 2>&1; then
    /usr/bin/codesign --force --deep --sign - "${APP_BUNDLE}"
fi

echo "========================================================================"
echo "Universal build successful! Artifacts ready in:"
echo " - Universal static archive: ${STATIC_UNI}"
echo " - Universal dylib:          ${DYLIB_UNI}"
echo " - Swift bridge folder:      ${BRIDGE_DIR}"
echo " - macOS App Bundle:         ${APP_BUNDLE}"
echo "========================================================================"
