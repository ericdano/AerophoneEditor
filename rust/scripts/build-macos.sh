#!/bin/bash
# Build "AE-30 Patch Editor.app" and a .dmg for macOS.
#
#   rust/scripts/build-macos.sh
#
# - Universal (Apple Silicon + Intel) if both Rust targets are installed.
# - Signed with a "Developer ID Application" certificate if the keychain has
#   one, otherwise ad-hoc (fine for your own Mac; others see a warning).
# - Notarized if NOTARY_PROFILE names a `xcrun notarytool store-credentials` profile.
# - Uses ae30-app/assets/icons/logo.icns as the icon when it exists.
#
# Output: rust/target/macos/AE-30 Patch Editor.app and AE30PatchEditor-<version>.dmg
set -euo pipefail

cd "$(dirname "$0")/.."            # the rust/ workspace
NAME="AE-30 Patch Editor"
EXE=AE30PatchEditor
BIN=ae30edit
OUT=target/macos
APP="$OUT/$NAME.app"
META=$(cargo metadata --no-deps --format-version 1)
VERSION=$(grep -o '"name":"ae30-app","version":"[^"]*"' <<<"$META" | sed 's/.*"version":"//; s/"$//')
[ -n "$VERSION" ] || { echo "Couldn't read the ae30-app version" >&2; exit 1; }
echo "Building $NAME $VERSION"

# --- compile ---
INSTALLED=$(rustup target list --installed)
BUILT=()
for T in aarch64-apple-darwin x86_64-apple-darwin; do
  if grep -qx "$T" <<<"$INSTALLED"; then
    echo "Compiling for $T..."
    cargo build --release --bin "$BIN" --target "$T"
    BUILT+=("target/$T/release/$BIN")
  fi
done
[ ${#BUILT[@]} -gt 0 ] || { echo "No macOS Rust targets installed" >&2; exit 1; }

# --- assemble the .app ---
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create "${BUILT[@]}" -output "$APP/Contents/MacOS/$EXE"
ICON_KEY=""
if [ -f ae30-app/assets/icons/logo.icns ]; then
  cp ae30-app/assets/icons/logo.icns "$APP/Contents/Resources/$EXE.icns"
  ICON_KEY="<key>CFBundleIconFile</key><string>$EXE</string>"
fi
cp ae30-app/assets/fonts/Inter-LICENSE.txt "$APP/Contents/Resources/"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>com.github.ericdano.ae30patcheditor</string>
  <key>CFBundleExecutable</key><string>$EXE</string>
  $ICON_KEY
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.music</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

# --- sign ---
IDENTITY=$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | sed -n 's/.*"\(.*\)"/\1/p' || true)
if [ -n "$IDENTITY" ]; then
  echo "Signing with: $IDENTITY"
  codesign --force --sign "$IDENTITY" --timestamp --options runtime "$APP"
else
  echo "No Developer ID certificate found: signing ad-hoc (for this Mac)."
  codesign --force --sign - "$APP"
fi
codesign --verify --strict "$APP"

# --- dmg ---
DMG="$OUT/$EXE-$VERSION.dmg"
rm -f "$DMG"
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "$NAME $VERSION" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"
[ -n "$IDENTITY" ] && codesign --force --sign "$IDENTITY" --timestamp "$DMG"
if [ -n "${NOTARY_PROFILE:-}" ] && [ -n "$IDENTITY" ]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

"$APP/Contents/MacOS/$EXE" --smoke
echo
echo "Done:"
echo "  App: rust/$APP   ($(lipo -archs "$APP/Contents/MacOS/$EXE"))"
echo "  DMG: rust/$DMG"
