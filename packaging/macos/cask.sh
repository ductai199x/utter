#!/bin/bash
# Packs a signed Utter.app for Homebrew: a zip, and the cask (utter.rb) with its version,
# checksum and download URL. On a Mac, after the app is built and signed (sign.sh).
#
# Usage: cask.sh path/to/Utter.app VERSION URL_BASE OUT_DIR
#   URL_BASE: where the zip will be downloaded from, e.g.
#   https://github.com/ductai199x/utter/releases/download/v0.1.9 (or file:///path for a test)
set -euo pipefail
app=${1:?usage: cask.sh Utter.app VERSION URL_BASE OUT_DIR}
version=${2:?version}
base=${3:?url base}
out=${4:?out dir}
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out"
zip="Utter-$version-macos-arm64.zip"
rm -f "$out/$zip"
# ditto keeps the bundle's symlinks, extended attributes and signature intact.
ditto -c -k --keepParent "$app" "$out/$zip"
sha=$(shasum -a 256 "$out/$zip" | cut -d' ' -f1)
sed -e "s|\"VERSION\"|\"$version\"|" -e "s|\"SHA256\"|\"$sha\"|" -e "s|\"URL\"|\"$base/$zip\"|" "$here/utter.rb" > "$out/utter.rb"
echo "$out/$zip"
echo "$out/utter.rb (sha256 $sha)"
