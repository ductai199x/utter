#!/bin/sh
# Fixes the AppImage `npx tauri build` makes (target/release/bundle/appimage), in place:
#
# 1. libwayland and libdrm come from the system it runs on, never the build machine: they go
#    with the system's Mesa, which the AppImage never carries. With older ones inside, WebKit
#    could not make its EGL display under Fedora 44's Mesa and the window stayed empty (a
#    sibling project, 2026-10-14). linuxdeploy as Tauri runs it takes no exclusions, so they are
#    removed from the AppDir and the image packed again.
# 2. The runtime in front of the image is swapped for AppImage's static type2 runtime: Tauri's
#    loads libfuse.so.2, which stock Ubuntu 24.04 and later do not install ("AppImages require
#    FUSE to run"). An AppImage is its runtime followed by a squashfs image.
#
#     sh packaging/linux/fix-appimage.sh target/release/bundle/appimage/Utter_0.1.7_amd64.AppImage
set -eu

RUNTIME_URL=https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
# The packer Tauri downloaded for its own build.
PACKER="$HOME/.cache/tauri/linuxdeploy-plugin-appimage.AppImage"

image=$(realpath "$1")
dir=$(dirname "$image")
appdir=$(find "$dir" -maxdepth 1 -name '*.AppDir' | head -n 1)
[ -n "$appdir" ] || { echo "no AppDir beside $image" >&2; exit 1; }
[ -x "$PACKER" ] || { echo "$PACKER is missing: Tauri downloads it with its first AppImage" >&2; exit 1; }

# 1. Without the system's libraries, packed again.
find "$appdir/usr/lib" -maxdepth 1 \( -name 'libwayland-*' -o -name 'libdrm*' \) -print -delete
rm -f "$image"
(cd "$dir" && ARCH=x86_64 OUTPUT="$image" LDAI_OUTPUT="$image" APPIMAGE_EXTRACT_AND_RUN=1 "$PACKER" --appdir "$appdir" >/dev/null)
[ -f "$image" ] || { echo "the AppImage was not packed again" >&2; exit 1; }

# 2. The static runtime, pinned and checked.
runtime="$HOME/.cache/utter-build/runtime-x86_64-$RUNTIME_SHA256"
if [ ! -f "$runtime" ]; then
  mkdir -p "$(dirname "$runtime")"
  curl -fsSL -o "$runtime.part" "$RUNTIME_URL"
  mv "$runtime.part" "$runtime"
fi
echo "$RUNTIME_SHA256  $runtime" | sha256sum -c --quiet - || { rm -f "$runtime"; echo "not the pinned runtime" >&2; exit 1; }

# Where the runtime's ELF file ends (its section headers come last): the squashfs image starts.
le() { od -An -t "u$2" -j "$1" -N "$2" "$image" | tr -d ' '; }
start=$(($(le 40 8) + $(le 58 2) * $(le 60 2)))
[ "$(tail -c +$((start + 1)) "$image" | head -c 4)" = hsqs ] || { echo "no squashfs image at $start" >&2; exit 1; }
{ cat "$runtime"; tail -c +$((start + 1)) "$image"; } > "$image.swap"
chmod 755 "$image.swap"
mv "$image.swap" "$image"
echo "$image: system libwayland and libdrm, static runtime"
