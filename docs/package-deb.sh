#!/usr/bin/env bash
# Package bettercut for Debian and Ubuntu: dist/bettercut_<version>_amd64.deb,
# installed with `sudo apt install ./bettercut_<version>_amd64.deb` and kept
# up to date by the system like any other package.
#
# Made from the AppDir docs/package-linux.sh leaves behind, so run that
# first: its binaries already carry FFmpeg and every other library they
# need, and look for them beside themselves ($ORIGIN/../lib). The package
# puts that whole tree in /opt/bettercut, with `bettercut` and
# `bettercut-mcp` in /usr/bin to start it, and names as dependencies only
# what is left to the system: the C library and the graphics, sound and
# window libraries that must be the system's own.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(grep -m1 '^version' "$REPO/Cargo.toml" | sed -E 's/.*"(.*)".*/\1/')"
DIST="$REPO/dist"
APPDIR="$DIST/bettercut.AppDir"
ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

if [ ! -x "$APPDIR/usr/bin/bettercut" ]; then
  echo "No AppDir at $APPDIR: run docs/package-linux.sh first" >&2
  exit 1
fi

# The program, as the AppImage holds it.
mkdir -p "$ROOT/opt/bettercut"
cp -a "$APPDIR/usr/." "$ROOT/opt/bettercut/"

# Started from anywhere: small launchers rather than links, so the program
# finds its libraries beside where it really is.
mkdir -p "$ROOT/usr/bin"
for name in bettercut bettercut-mcp; do
  printf '#!/bin/sh\nexec /opt/bettercut/bin/%s "$@"\n' "$name" > "$ROOT/usr/bin/$name"
  chmod 755 "$ROOT/usr/bin/$name"
done

# In the applications menu, with its icon, opening .vproj projects.
mkdir -p "$ROOT/usr/share/applications" "$ROOT/usr/share/icons/hicolor/256x256/apps"
sed 's|^Exec=.*|Exec=/opt/bettercut/bin/bettercut %f|' \
  "$APPDIR/usr/share/applications/bettercut.desktop" \
  > "$ROOT/usr/share/applications/bettercut.desktop"
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/bettercut.png" \
  "$ROOT/usr/share/icons/hicolor/256x256/apps/bettercut.png"
mkdir -p "$ROOT/usr/share/doc/bettercut"
cp "$APPDIR/usr/share/licenses/bettercut/"* "$ROOT/usr/share/doc/bettercut/"

# What the system must provide: every library the binaries load that is not
# in the package, named by the package it comes from — plus the ones loaded
# by name at run time, which no linker list shows (the Vulkan loader, and
# the X11 and Wayland window libraries).
LINKED="$(LD_LIBRARY_PATH="$ROOT/opt/bettercut/lib" ldd \
    "$ROOT/opt/bettercut/bin/bettercut" "$ROOT/opt/bettercut/bin/bettercut-mcp" \
  | awk '/=> \//{print $3}' | grep -v "^$ROOT" | sort -u \
  | xargs -r dpkg -S 2>/dev/null | cut -d: -f1 | sort -u | grep -v '^libc6$' || true)"
GLIBC="$(ldd --version | head -1 | grep -oE '[0-9]+\.[0-9]+$')"
DEPENDS="libc6 (>= $GLIBC)"
for package in $LINKED; do
  DEPENDS="$DEPENDS, $package"
done
DEPENDS="$DEPENDS, libvulkan1, libxkbcommon0, libx11-6 | libwayland-client0"
SIZE="$(du -sk "$ROOT/opt" "$ROOT/usr" | awk '{total += $1} END {print total}')"

mkdir -p "$ROOT/DEBIAN"
cat > "$ROOT/DEBIAN/control" <<CONTROL
Package: bettercut
Version: $VERSION
Section: video
Priority: optional
Architecture: amd64
Maintainer: bettercut <https://github.com/mentiacademyedu/bettercut/issues>
Installed-Size: $SIZE
Depends: $DEPENDS
Homepage: https://bettercut.dev
Description: Free, open-source video editor
 A desktop video editor with CapCut-style convenience, built in Rust to
 stay quick on ordinary laptops: cut, titles, effects, transitions, colour,
 sound and export, and an MCP server so an AI assistant can edit too.
CONTROL

OUTPUT="$DIST/bettercut_${VERSION}_amd64.deb"
rm -f "$OUTPUT"
dpkg-deb --build --root-owner-group "$ROOT" "$OUTPUT"
dpkg-deb --info "$OUTPUT"
echo "Package: $OUTPUT"
