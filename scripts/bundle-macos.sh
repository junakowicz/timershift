#!/usr/bin/env bash
# Builds target/release/bundle/TimerShift.app.
# macOS only delivers notifications for real app bundles, so use this rather
# than the bare binary for everyday use.
#
# --universal builds one binary for both Apple Silicon and Intel Macs (used for
# releases); it needs `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME=TimerShift
BUNDLE_ID=com.timershift.TimerShift # keep in sync with src/notify.rs
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
APP=target/release/bundle/$APP_NAME.app

if [[ "${1:-}" == "--universal" ]]; then
  cargo build --release --target aarch64-apple-darwin
  cargo build --release --target x86_64-apple-darwin
  BIN=target/universal/timershift
  mkdir -p "$(dirname "$BIN")"
  lipo -create -output "$BIN" \
    target/aarch64-apple-darwin/release/timershift \
    target/x86_64-apple-darwin/release/timershift
else
  cargo build --release
  BIN=target/release/timershift
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/timershift"

# Render the app's own icon at 1024px and let iconutil turn it into an .icns.
ICONSET=$(mktemp -d)/AppIcon.iconset
mkdir -p "$ICONSET"
BIG="$ICONSET/icon_512x512@2x.png"
"$BIN" --write-icon "$BIG"
for size in 16 32 128 256; do
  sips -z $size $size "$BIG" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$BIG" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
sips -z 512 512 "$BIG" --out "$ICONSET/icon_512x512.png" >/dev/null
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>timershift</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature so macOS accepts the bundle's identity locally.
codesign --force --deep --sign - "$APP"

echo "Built $APP"
echo "Install with: cp -R \"$APP\" /Applications/"
