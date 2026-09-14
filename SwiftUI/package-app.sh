#!/bin/sh
# package-app.sh — build, stage the binary into LAC Studio.app, re-sign.
# Re-signing is mandatory after every copy: a stale ad-hoc signature
# makes launchd refuse spawn (RBSRequestErrorDomain Code=5).
# Usage: ./package-app.sh [--release] [--open] [--restart]
set -eu
cd "$(dirname "$0")"

CONFIG="debug"
DO_OPEN=0
DO_RESTART=0

for arg in "$@"; do
    case "$arg" in
        --release|-r) CONFIG="release" ;;
        --open|-o) DO_OPEN=1 ;;
        --restart) DO_RESTART=1 ;;
    esac
done

if [ "$CONFIG" = "release" ]; then
    echo "Building LAC Studio in Release mode..."
    swift build -c release
    BIN="$(swift build -c release --show-bin-path)/LoopLACStudio"
else
    swift build
    BIN="$(swift build --show-bin-path)/LoopLACStudio"
fi
APP_DIR=".build/LAC Studio.app"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"
cat << 'EOF' > "$APP_DIR/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>LoopLACStudio</string>
    <key>CFBundleIdentifier</key>
    <string>org.lac.studio</string>
    <key>CFBundleName</key>
    <string>Loop LAC Studio</string>
    <key>CFBundleDisplayName</key>
    <string>Loop LAC Studio</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>2.8</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF
cp "$BIN" "$APP_DIR/Contents/MacOS/LoopLACStudio"
# Drop the pre-rename binary if present (same bundle id would otherwise
# re-activate the stale process on `open` instead of launching fresh).
rm -f "$APP_DIR/Contents/MacOS/LACStudio"
# Generate AppIcon.icns if needed
if [ -f Resources/LACDashboard.appiconset/icon_1024.png ]; then
    mkdir -p .build/icon/AppIcon.iconset
    sips -z 16 16     Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_16x16.png >/dev/null 2>&1 || true
    sips -z 32 32     Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_16x16@2x.png >/dev/null 2>&1 || true
    sips -z 32 32     Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_32x32.png >/dev/null 2>&1 || true
    sips -z 64 64     Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_32x32@2x.png >/dev/null 2>&1 || true
    sips -z 128 128   Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_128x128.png >/dev/null 2>&1 || true
    sips -z 256 256   Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_128x128@2x.png >/dev/null 2>&1 || true
    sips -z 256 256   Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_256x256.png >/dev/null 2>&1 || true
    sips -z 512 512   Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_256x256@2x.png >/dev/null 2>&1 || true
    sips -z 512 512   Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_512x512.png >/dev/null 2>&1 || true
    sips -z 1024 1024 Resources/LACDashboard.appiconset/icon_1024.png --out .build/icon/AppIcon.iconset/icon_512x512@2x.png >/dev/null 2>&1 || true
    iconutil -c icns .build/icon/AppIcon.iconset -o .build/icon/AppIcon.icns >/dev/null 2>&1 || true
fi
# Copy AppIcon.icns into app bundle Resources
cp .build/icon/AppIcon.icns "$APP_DIR/Contents/Resources/AppIcon.icns" 2>/dev/null || true
# Copy resource bundles (Assets.xcassets, etc.) into the app bundle
cp -R Resources/LACDashboard.appiconset "$APP_DIR/Contents/Resources/LACDashboard.appiconset" 2>/dev/null || true
codesign --force --deep -s - "$APP_DIR"
# Also maintain aliases for scripts expecting Loop LAC Studio.app and LACDashboard.app
rm -rf ".build/Loop LAC Studio.app" ".build/LACDashboard.app"
cp -R "$APP_DIR" ".build/Loop LAC Studio.app"
codesign --force --deep -s - ".build/Loop LAC Studio.app"
cp -R "$APP_DIR" ".build/LACDashboard.app"
codesign --force --deep -s - ".build/LACDashboard.app"
echo "staged + signed: $APP_DIR"
if [ "$DO_OPEN" -eq 1 ]; then
    open "$APP_DIR"
fi
if [ "$DO_RESTART" -eq 1 ]; then
    pkill -f "Contents/MacOS/LoopLACStudio" 2>/dev/null || true
    pkill -f "Contents/MacOS/LACDashboard" 2>/dev/null || true
    pkill -f "Contents/MacOS/LACStudio" 2>/dev/null || true
    sleep 1
    open "$APP_DIR"
fi
