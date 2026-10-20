#!/bin/bash
# Packs a signed Utter.app for Homebrew: the zip a release carries, and Casks/utter.rb (this
# repository is its own tap) with the version, the zip's checksum and its download URL. On a Mac,
# after the app is built and signed (sign.sh); then upload the zip to the release and commit
# Casks/utter.rb (packaging/macos/README.md).
#
# Usage: cask.sh path/to/Utter.app VERSION OUT_DIR [URL_BASE]
#   URL_BASE: where the zip is downloaded from; default the GitHub release vVERSION. A file://
#   folder tests the cask before anything is published.
set -euo pipefail
app=${1:?usage: cask.sh Utter.app VERSION OUT_DIR [URL_BASE]}
version=${2:?version}
out=${3:?out dir}
base=${4:-https://github.com/ductai199x/utter/releases/download/v$version}
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
mkdir -p "$out" "$repo/Casks"
zip="Utter-$version-macos-arm64.zip"
rm -f "$out/$zip"
# The in-app update replaces Utter.app with this archive (packaging/latest-json.sh signs it).
tar -czf "$out/Utter-$version-macos-arm64.app.tar.gz" -C "$(dirname "$app")" "$(basename "$app")"
# ditto keeps the bundle's symlinks, extended attributes and signature intact.
ditto -c -k --keepParent "$app" "$out/$zip"
sha=$(shasum -a 256 "$out/$zip" | cut -d' ' -f1)
sed -e "s|\"VERSION\"|\"$version\"|" -e "s|\"SHA256\"|\"$sha\"|" -e "s|\"URL\"|\"$base/$zip\"|" \
    "$here/utter.rb" > "$repo/Casks/utter.rb"
echo "$out/$zip"
echo "$repo/Casks/utter.rb (sha256 $sha)"
