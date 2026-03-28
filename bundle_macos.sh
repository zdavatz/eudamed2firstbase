#!/bin/bash
# Build a macOS .app bundle for eudamed2firstbase
#
# Usage:
#   ./bundle_macos.sh                  # Build native release (no signing)
#   ./bundle_macos.sh --universal      # Build universal binary (arm64 + x86_64)
#   ./bundle_macos.sh --sign           # Universal + code sign (requires SIGNING_IDENTITY)
#   ./bundle_macos.sh --dmg            # Universal + sign + create DMG
#   ./bundle_macos.sh --notarize       # Universal + sign + DMG + notarize
#
# Environment variables for signing/notarization:
#   SIGNING_IDENTITY    "Developer ID Application: Name (TEAMID)"
#   APPLE_ID            Apple ID email for notarization
#   APPLE_TEAM_ID       Team ID
#   APPLE_APP_PASSWORD  App-specific password for notarization
set -e

APP_NAME="eudamed2firstbase"
VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
BUNDLE_DIR="target/release/${APP_NAME}.app"

# Parse flags
UNIVERSAL=false
SIGN=false
DMG=false
NOTARIZE=false
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=true ;;
        --sign)      UNIVERSAL=true; SIGN=true ;;
        --dmg)       UNIVERSAL=true; SIGN=true; DMG=true ;;
        --notarize)  UNIVERSAL=true; SIGN=true; DMG=true; NOTARIZE=true ;;
    esac
done

# --- Build ---
if $UNIVERSAL; then
    echo "=== Building universal binary (arm64 + x86_64) ==="
    rustup target add aarch64-apple-darwin x86_64-apple-darwin 2>/dev/null || true
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    BINARY="target/release/${APP_NAME}-universal"
    lipo -create \
        "target/aarch64-apple-darwin/release/${APP_NAME}" \
        "target/x86_64-apple-darwin/release/${APP_NAME}" \
        -output "$BINARY"
    echo "  Universal binary: $BINARY"
else
    echo "=== Building release binary ==="
    cargo build --release
    BINARY="target/release/${APP_NAME}"
fi

# --- Create .app bundle ---
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/Contents/MacOS"
mkdir -p "$BUNDLE_DIR/Contents/Resources"

cp "$BINARY" "$BUNDLE_DIR/Contents/MacOS/${APP_NAME}"
cp assets/icon.icns "$BUNDLE_DIR/Contents/Resources/AppIcon.icns"

cat > "$BUNDLE_DIR/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>com.ywesee.${APP_NAME}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.business</string>
</dict>
</plist>
PLIST

echo "Built: $BUNDLE_DIR"

# --- Code Sign ---
if $SIGN; then
    if [ -z "$SIGNING_IDENTITY" ]; then
        echo "WARNING: SIGNING_IDENTITY not set, skipping code signing"
        SIGN=false
    else
        echo "=== Code signing ==="
        codesign --force --options runtime \
            --sign "$SIGNING_IDENTITY" \
            --entitlements entitlements.plist \
            --deep \
            "$BUNDLE_DIR"
        echo "  Signed with: $SIGNING_IDENTITY"
        codesign --verify --verbose "$BUNDLE_DIR"
    fi
fi

# --- Create DMG ---
if $DMG; then
    DMG_NAME="${APP_NAME}-${VERSION}-macos-universal.dmg"
    echo "=== Creating DMG ==="

    # Check for create-dmg (pretty DMG with drag-to-Applications)
    if command -v create-dmg &>/dev/null; then
        rm -f "$DMG_NAME"
        create-dmg \
            --volname "$APP_NAME" \
            --window-pos 200 120 \
            --window-size 600 400 \
            --icon-size 100 \
            --icon "${APP_NAME}.app" 150 190 \
            --app-drop-link 450 190 \
            "$DMG_NAME" \
            "$BUNDLE_DIR" || true
    else
        # Fallback: simple DMG with Applications symlink
        STAGING=$(mktemp -d)
        cp -R "$BUNDLE_DIR" "$STAGING/"
        ln -s /Applications "$STAGING/Applications"
        rm -f "$DMG_NAME"
        hdiutil create -volname "$APP_NAME" \
            -srcfolder "$STAGING" \
            -ov -format UDZO \
            "$DMG_NAME"
        rm -rf "$STAGING"
    fi

    echo "  DMG: $DMG_NAME"

    # Staple notarization ticket to DMG if already notarized
    if $NOTARIZE && $SIGN; then
        : # Notarization happens below
    fi
fi

# --- Notarize ---
if $NOTARIZE; then
    if [ -z "$APPLE_ID" ] || [ -z "$APPLE_TEAM_ID" ] || [ -z "$APPLE_APP_PASSWORD" ]; then
        echo "WARNING: Notarization env vars not set (APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD)"
    else
        echo "=== Notarizing ==="
        xcrun notarytool submit "$DMG_NAME" \
            --apple-id "$APPLE_ID" \
            --team-id "$APPLE_TEAM_ID" \
            --password "$APPLE_APP_PASSWORD" \
            --wait
        xcrun stapler staple "$DMG_NAME"
        echo "  Notarized and stapled: $DMG_NAME"
    fi
fi

echo ""
echo "Done!"
if $DMG; then
    echo "  Distribute: $DMG_NAME"
else
    echo "  Run with: open $BUNDLE_DIR"
fi
