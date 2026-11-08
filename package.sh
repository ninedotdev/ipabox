#!/bin/bash
# Build ipabox.app. Assembles the bundle by hand (no cargo-bundle): release
# binary + Info.plist + icns + the plumesign helper, then ad-hoc codesigns it.

set -euo pipefail

cd "$(dirname "$0")"
APP="dist/ipabox.app"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"

echo "==> building release binary"
cargo build --release

echo "==> generating icon"
python3 tools/make_icon.py

ICONSET="$(mktemp -d)/ipabox.iconset"
mkdir -p "$ICONSET"
gen() { sips -z "$1" "$1" assets/ipabox-icon-1024.png --out "$ICONSET/$2" >/dev/null; }
gen 16   icon_16x16.png
gen 32   icon_16x16@2x.png
gen 32   icon_32x32.png
gen 64   icon_32x32@2x.png
gen 128  icon_128x128.png
gen 256  icon_128x128@2x.png
gen 256  icon_256x256.png
gen 512  icon_256x256@2x.png
gen 512  icon_512x512.png
cp assets/ipabox-icon-1024.png "$ICONSET/icon_512x512@2x.png"
iconutil -c icns "$ICONSET" -o assets/ipabox.icns

echo "==> assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/ipabox "$APP/Contents/MacOS/ipabox"
chmod +x "$APP/Contents/MacOS/ipabox"
cp assets/ipabox.icns "$APP/Contents/Resources/ipabox.icns"

# plumesign does the Apple-ID signing. It is vendored rather than embedded in
# the binary (the Go app embedded all 84 MB of it into its executable).
if [ -f vendor/plumesign ]; then
  cp vendor/plumesign "$APP/Contents/Resources/plumesign"
  chmod +x "$APP/Contents/Resources/plumesign"
else
  echo "    warning: vendor/plumesign missing — signing will fail at runtime"
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>ipabox</string>
  <key>CFBundleDisplayName</key><string>ipabox</string>
  <key>CFBundleExecutable</key><string>ipabox</string>
  <key>CFBundleIdentifier</key><string>com.ipabox.app</string>
  <key>CFBundleIconFile</key><string>ipabox</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <key>NSLocalNetworkUsageDescription</key><string>ipabox discovers Apple TVs and iOS devices on your local network to pair with them and sideload apps.</string>
  <key>NSBonjourServices</key>
  <array>
    <string>_remotepairing._tcp</string>
    <string>_remotepairing-manual-pairing._tcp</string>
    <string>_apple-mobdev2._tcp</string>
  </array>
</dict>
</plist>
PLIST

echo "==> codesigning (ad-hoc)"
codesign --force --deep --sign - "$APP"
codesign --verify --verbose "$APP" 2>&1 | tail -1

echo "==> done: $APP ($(du -sh "$APP" | cut -f1))"
