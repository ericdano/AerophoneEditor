#!/bin/bash
# Build the AE-30 Patch Editor for Windows 10/11 (64-bit) on macOS, as a zip.
#
#   rust/scripts/build-windows.sh
#
# Needs, once:  rustup target add x86_64-pc-windows-gnu ; brew install mingw-w64
# Output: rust/target/windows/AE30PatchEditor-<version>-windows-x64.zip
# The .exe can't run on macOS: try it on a Windows PC before sharing.
set -euo pipefail

cd "$(dirname "$0")/.."
TARGET=x86_64-pc-windows-gnu
BIN=ae30edit
OUT=target/windows
META=$(cargo metadata --no-deps --format-version 1)
VERSION=$(grep -o '"name":"ae30-app","version":"[^"]*"' <<<"$META" | sed 's/.*"version":"//; s/"$//')
[ -n "$VERSION" ] || { echo "Couldn't read the ae30-app version" >&2; exit 1; }
rustup target list --installed | grep -qx "$TARGET" || { echo "Run: rustup target add $TARGET" >&2; exit 1; }
command -v x86_64-w64-mingw32-gcc >/dev/null || { echo "Run: brew install mingw-w64" >&2; exit 1; }

cargo build --release --bin "$BIN" --target "$TARGET"

STAGE="$OUT/AE30PatchEditor"
rm -rf "$STAGE"
mkdir -p "$STAGE"
cp "target/$TARGET/release/$BIN.exe" "$STAGE/AE30PatchEditor.exe"
cp ae30-app/assets/fonts/Inter-LICENSE.txt "$STAGE/"
sed 's/$/\r/' > "$STAGE/README.txt" <<TXT
AE-30 Patch Editor $VERSION
Scene and tone editor for the Roland Aerophone Pro AE-30.

Double-click AE30PatchEditor.exe. No installation is needed.

The first time, Windows may say "Windows protected your PC" because the app
isn't signed with a paid certificate: click "More info", then "Run anyway".

Connect the AE-30 by USB. If it doesn't appear, install Roland's AE-30 USB
driver or set the AE-30's USB driver mode to Generic. Close other music
software that uses the AE-30 (Windows lets one program use a MIDI port).

Command-line tools: AE30PatchEditor.exe help
TXT
ZIP="$OUT/AE30PatchEditor-$VERSION-windows-x64.zip"
rm -f "$ZIP"
(cd "$OUT" && zip -q -r -X "$(basename "$ZIP")" AE30PatchEditor)
echo
echo "Done: rust/$ZIP"
