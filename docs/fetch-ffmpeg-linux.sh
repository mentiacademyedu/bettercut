#!/usr/bin/env bash
# Build the FFmpeg bettercut links against on Linux: the same version as the
# Windows SDK and the Mac build (docs/ffmpeg.md), LGPL only, shared libraries,
# into vendor/ffmpeg-linux (or the folder given as $1).
#
# Built from the released source, checked against its SHA-256. On Debian or
# Ubuntu it needs:
#
#   sudo apt-get install build-essential nasm pkg-config \
#     libopenh264-dev libzimg-dev libdav1d-dev libvpx-dev libva-dev
#
#   openh264  the H.264 encoder proxies are made with (BSD)
#   zimg      the zscale filter HDR footage is tone-mapped through (WTFPL)
#   dav1d     AV1 decoding (BSD)
#   libvpx    VP9, the codec of transparent (alpha) exports (BSD)
#   libva     VAAPI, Intel and AMD hardware encoding (MIT)
set -euo pipefail

VERSION="8.1.2"
SHA256="464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c"
URL="https://ffmpeg.org/releases/ffmpeg-${VERSION}.tar.xz"

REPO="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${1:-$REPO/vendor/ffmpeg-linux}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "FFmpeg $VERSION -> $PREFIX"
curl -sSfL "$URL" -o "$WORK/ffmpeg.tar.xz"
echo "$SHA256  $WORK/ffmpeg.tar.xz" | sha256sum -c -
tar -xJf "$WORK/ffmpeg.tar.xz" -C "$WORK"
cd "$WORK/ffmpeg-$VERSION"

# LGPL v3, shared, no programs. Never --enable-gpl or --enable-nonfree: the
# check after the build refuses the result if either slipped in. No X11 grab
# or SDL: bettercut never uses them.
./configure \
  --prefix="$PREFIX" \
  --enable-version3 \
  --enable-shared --disable-static \
  --disable-programs --disable-doc --disable-debug \
  --enable-vaapi \
  --enable-libopenh264 --enable-libzimg --enable-libdav1d --enable-libvpx \
  --disable-libx264 --disable-libx265 --disable-libfdk-aac \
  --disable-xlib --disable-libxcb --disable-sdl2

make -j"$(nproc)"
rm -rf "$PREFIX"
make install

# The licence guard, on what was actually built rather than on the flags we
# meant to pass: the configure line FFmpeg records.
CONFIG="$(grep -h 'FFMPEG_CONFIGURATION' config.h)"
for forbidden in --enable-gpl --enable-nonfree --enable-libx264 --enable-libx265; do
  if echo "$CONFIG" | grep -q -- "$forbidden"; then
    echo "Refusing: this FFmpeg was built with $forbidden" >&2
    rm -rf "$PREFIX"
    exit 1
  fi
done
cp LICENSE.md COPYING.LGPLv3 "$PREFIX/" 2>/dev/null || true
echo "FFmpeg $VERSION installed at $PREFIX (LGPL v3, shared)."
