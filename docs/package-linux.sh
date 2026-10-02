#!/usr/bin/env bash
# Package bettercut for Linux as an AppImage: dist/bettercut-<version>-x86_64.AppImage,
# one file that runs on most distributions without installing.
#
# Needs the FFmpeg from docs/fetch-ffmpeg-linux.sh and linuxdeploy (the
# AppImage tool that copies in every library the app loads and points the app
# at those copies):
#
#   curl -sSfL -o linuxdeploy https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
#   chmod +x linuxdeploy
#
# and the path to it in LINUXDEPLOY (default: ./linuxdeploy).
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
FFMPEG="${FFMPEG_PREFIX:-$REPO/vendor/ffmpeg-linux}"
LINUXDEPLOY="${LINUXDEPLOY:-$REPO/linuxdeploy}"
VERSION="$(grep -m1 '^version' "$REPO/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
DIST="$REPO/dist"
APPDIR="$DIST/bettercut.AppDir"

if [ ! -d "$FFMPEG/lib" ]; then
  echo "No FFmpeg at $FFMPEG: run docs/fetch-ffmpeg-linux.sh first" >&2
  exit 1
fi

export FFMPEG_INCLUDE_DIR="$FFMPEG/include"
export FFMPEG_LIBS_DIR="$FFMPEG/lib"
export FFMPEG_LINK_MODE=dynamic
export BETTERCUT_FFMPEG_BIN="$FFMPEG/lib"
(cd "$REPO" && cargo build --release -p bettercut-desktop -p bettercut-mcp)
BIN="$REPO/target/release/bettercut"

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
  "$APPDIR/usr/share/icons/hicolor/256x256/apps" "$APPDIR/usr/share/licenses/bettercut"
cp "$BIN" "$APPDIR/usr/bin/bettercut"
# The MCP server, for AI assistants, beside the app.
cp "$REPO/target/release/bettercut-mcp" "$APPDIR/usr/bin/bettercut-mcp"

# The icon, from the same drawing the window uses. Run with FFmpeg findable,
# since the binary links it even to draw an icon.
ICONS="$(mktemp -d)"
trap 'rm -rf "$ICONS"' EXIT
LD_LIBRARY_PATH="$FFMPEG/lib:${LD_LIBRARY_PATH:-}" BETTERCUT_WRITE_ICONS="$ICONS" "$BIN"
cp "$ICONS/icon_256.png" "$APPDIR/usr/share/icons/hicolor/256x256/apps/bettercut.png"

cat > "$APPDIR/usr/share/applications/bettercut.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=bettercut
Comment=Free, open-source video editor
Exec=bettercut %f
Icon=bettercut
Categories=AudioVideo;Video;AudioVideoEditing;
MimeType=application/x-bettercut-project;
Terminal=false
DESKTOP

# The licences, as on Windows and Mac.
cp "$REPO/LICENSE-MIT" "$REPO/LICENSE-APACHE" "$APPDIR/usr/share/licenses/bettercut/"
cp "$FFMPEG"/LICENSE.md "$FFMPEG"/COPYING.LGPLv3 "$APPDIR/usr/share/licenses/bettercut/" 2>/dev/null || true
(cd "$REPO" && cargo about generate -c docs/about.toml docs/about.hbs \
  -o "$APPDIR/usr/share/licenses/bettercut/THIRD-PARTY-NOTICES.txt")

# linuxdeploy copies FFmpeg and everything it and the app need (but not the
# graphics and sound drivers, which must be the system's own), and sets each
# binary to look beside itself first.
OUTPUT="$DIST/bettercut-${VERSION}-x86_64.AppImage"
rm -f "$OUTPUT"
LD_LIBRARY_PATH="$FFMPEG/lib:${LD_LIBRARY_PATH:-}" \
  LDAI_OUTPUT="$OUTPUT" \
  APPIMAGE_EXTRACT_AND_RUN=1 \
  "$LINUXDEPLOY" --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/bettercut" \
    --executable "$APPDIR/usr/bin/bettercut-mcp" \
    --desktop-file "$APPDIR/usr/share/applications/bettercut.desktop" \
    --icon-file "$APPDIR/usr/share/icons/hicolor/256x256/apps/bettercut.png" \
    --output appimage
chmod +x "$OUTPUT"
echo "AppImage: $OUTPUT"
