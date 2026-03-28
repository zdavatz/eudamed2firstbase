#!/bin/bash
# Build a macOS .app bundle for eudamed2firstbase
set -e

APP_NAME="eudamed2firstbase"
BUNDLE_DIR="target/release/${APP_NAME}.app"

# Build release binary
cargo build --release

# Create .app bundle structure
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS"
mkdir -p "$BUNDLE_DIR/Contents/Resources"

# Copy binary
cp "target/release/${APP_NAME}" "$BUNDLE_DIR/Contents/MacOS/${APP_NAME}"

# Copy icon
cp assets/icon.icns "$BUNDLE_DIR/Contents/Resources/AppIcon.icns"

# Create Info.plist
cat > "$BUNDLE_DIR/Contents/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>eudamed2firstbase</string>
    <key>CFBundleDisplayName</key>
    <string>eudamed2firstbase</string>
    <key>CFBundleIdentifier</key>
    <string>com.ywesee.eudamed2firstbase</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleExecutable</key>
    <string>eudamed2firstbase</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
</dict>
</plist>
PLIST

echo "Built: $BUNDLE_DIR"
echo "   Run with: open $BUNDLE_DIR"
