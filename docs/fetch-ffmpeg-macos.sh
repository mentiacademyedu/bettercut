#!/usr/bin/env bash
# Build the FFmpeg bettercut links against on macOS: the same version as the
# Windows SDK (docs/ffmpeg.md), LGPL only, shared libraries, into
# vendor/ffmpeg-macos (or the folder given as $1).
#
# There is no ready-made LGPL macOS build to pin the way BtbN's is pinned for
# Windows, so this builds one from the released source, checked against its
# SHA-256. It needs Homebrew for four libraries, all BSD-style licensed:
#
#   brew install pkg-config openh264 zimg dav1d libvpx
#
#   openh264  the H.264 encoder proxies are made with (BSD)
#   zimg      the zscale filter HDR footage is tone-mapped through (WTFPL)
#   dav1d     AV1 decoding (BSD)
#   libvpx    VP9, the codec of transparent (alpha) exports (BSD)
#
# H.264/H.265 export on a Mac goes through Apple's VideoToolbox, which is part
# of the system: nothing to build or ship.
set -euo pipefail

VERSION="8.1.2"
SHA256="464beb5e7bf0c311e68b45ae2f04e9cc2af88851abb4082231742a74d97b524c"
URL="https://ffmpeg.org/releases/ffmpeg-${VERSION}.tar.xz"

REPO="$(cd "$(dirname "$0")/.." && pwd)"
PREFIX="${1:-$REPO/vendor/ffmpeg-macos}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "FFmpeg $VERSION -> $PREFIX"
curl -sSfL "$URL" -o "$WORK/ffmpeg.tar.xz"
echo "$SHA256  $WORK/ffmpeg.tar.xz" | shasum -a 256 -c -
tar -xJf "$WORK/ffmpeg.tar.xz" -C "$WORK"
cd "$WORK/ffmpeg-$VERSION"

# LGPL v3, shared, no programs. Never --enable-gpl or --enable-nonfree: the
# check after the build refuses the result if either slipped in.
# install_name_dir @rpath: each library names itself relative to whatever
# loads it, so the same files work from the build folder and inside the app.
./configure \
  --prefix="$PREFIX" \
  --install-name-dir=@rpath \
  --enable-version3 \
  --enable-shared --disable-static \
  --disable-programs --disable-doc --disable-debug \
  --enable-videotoolbox --enable-audiotoolbox \
  --enable-libopenh264 --enable-libzimg --enable-libdav1d --enable-libvpx \
  --disable-libx264 --disable-libx265 --disable-libfdk-aac

make -j"$(sysctl -n hw.ncpu)"
rm -rf "$PREFIX"
make install

# The licence guard, on what was actually built rather than on the flags we
# meant to pass: the configure line FFmpeg records in its own headers.
CONFIG="$(grep -h 'FFMPEG_CONFIGURATION' "$PREFIX"/include/libavutil/*.h 2>/dev/null || true)"
if [ -z "$CONFIG" ]; then
  CONFIG="$(grep -h 'FFMPEG_CONFIGURATION' config.h)"
fi
for forbidden in --enable-gpl --enable-nonfree --enable-libx264 --enable-libx265; do
  if echo "$CONFIG" | grep -q -- "$forbidden"; then
    echo "Refusing: this FFmpeg was built with $forbidden" >&2
    rm -rf "$PREFIX"
    exit 1
  fi
done
cp LICENSE.md COPYING.LGPLv3 "$PREFIX/" 2>/dev/null || true
echo "FFmpeg $VERSION installed at $PREFIX (LGPL v3, shared)."
