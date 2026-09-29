#!/usr/bin/env bash
# Package bettercut for macOS: dist/bettercut.app and dist/bettercut-<version>-macos.dmg.
#
# Needs the FFmpeg from docs/fetch-ffmpeg-macos.sh and, from Homebrew,
# dylibbundler (copies every non-system library the app loads into the bundle
# and points the app at those copies):
#
#   brew install dylibbundler
#
# The app is signed ad hoc, not with an Apple Developer ID, so it is not
# notarised: on first launch macOS refuses to open it until it is allowed in
# System Settings -> Privacy & Security ("Open Anyway"). Notarisation needs a
# paid Apple Developer account.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
FFMPEG="${FFMPEG_PREFIX:-$REPO/vendor/ffmpeg-macos}"
VERSION="$(grep -m1 '^version' "$REPO/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
DIST="$REPO/dist"
APP="$DIST/bettercut.app"

if [ ! -d "$FFMPEG/lib" ]; then
  echo "No FFmpeg at $FFMPEG: run docs/fetch-ffmpeg-macos.sh first" >&2
  exit 1
fi

# The release build, linked against that FFmpeg.
export FFMPEG_INCLUDE_DIR="$FFMPEG/include"
export FFMPEG_LIBS_DIR="$FFMPEG/lib"
export FFMPEG_LINK_MODE=dynamic
export BETTERCUT_FFMPEG_BIN="$FFMPEG/lib"
(cd "$REPO" && cargo build --release -p bettercut-desktop)
BIN="$REPO/target/release/bettercut"

# The bundle.
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$APP/Contents/Frameworks"
cp "$BIN" "$APP/Contents/MacOS/bettercut"

# Every library it loads that is not part of macOS: FFmpeg's, and the
# Homebrew ones FFmpeg uses in turn.
dylibbundler -od -b \
  -x "$APP/Contents/MacOS/bettercut" \
  -d "$APP/Contents/Frameworks" \
  -p @executable_path/../Frameworks/ \
  -s "$FFMPEG/lib"

# The icon, from the same drawing the window uses.
ICONS="$(mktemp -d)"
trap 'rm -rf "$ICONS"' EXIT
# Run from inside the bundle: that copy finds FFmpeg in Frameworks, which the
# one in target/ would not.
BETTERCUT_WRITE_ICONS="$ICONS/png" "$APP/Contents/MacOS/bettercut"
SET="$ICONS/bettercut.iconset"
mkdir -p "$SET"
for size in 16 32 128 256 512; do
  cp "$ICONS/png/icon_${size}.png" "$SET/icon_${size}x${size}.png"
  double=$((size * 2))
  cp "$ICONS/png/icon_${double}.png" "$SET/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$SET" -o "$APP/Contents/Resources/bettercut.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>bettercut</string>
  <key>CFBundleDisplayName</key><string>bettercut</string>
  <key>CFBundleIdentifier</key><string>dev.bettercut.editor</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleExecutable</key><string>bettercut</string>
  <key>CFBundleIconFile</key><string>bettercut</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>bettercut records voice-overs from your microphone.</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>bettercut project</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>CFBundleTypeExtensions</key><array><string>vproj</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST

# The licences, as on Windows.
mkdir -p "$APP/Contents/Resources/licences"
cp "$REPO/LICENSE-MIT" "$REPO/LICENSE-APACHE" "$APP/Contents/Resources/licences/"
cp "$FFMPEG"/LICENSE.md "$FFMPEG"/COPYING.LGPLv3 "$APP/Contents/Resources/licences/" 2>/dev/null || true
(cd "$REPO" && cargo about generate -c docs/about.toml docs/about.hbs \
  -o "$APP/Contents/Resources/licences/THIRD-PARTY-NOTICES.txt")

# Signed ad hoc: Apple Silicon will not run an unsigned binary at all.
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

# The disk image: the app and a shortcut to Applications to drag it onto.
DMG="$DIST/bettercut-${VERSION}-macos.dmg"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "bettercut ${VERSION}" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"
echo "Disk image: $DMG"
