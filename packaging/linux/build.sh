#!/bin/sh
# Builds Utter's Linux packages, the .deb and the AppImage, into target/release/bundle. On
# Ubuntu 24.04 (glibc 2.39: the packages run on Ubuntu 24.04+, Fedora 40+, Debian 13), from the
# repository's root:
#
#     sh packaging/linux/build.sh
#
# The release profile's line tables (backtraces with file and line; Windows keeps them in its
# PDB) are 70 MB inside a Linux binary: stripped here, function names kept.
set -eu
cd "$(dirname "$0")/../.."
(cd app && npm ci --silent && CARGO_PROFILE_RELEASE_STRIP=debuginfo npx tauri build)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
sh packaging/linux/fix-appimage.sh "target/release/bundle/appimage/Utter_${version}_amd64.AppImage"
ls -l target/release/bundle/deb/*.deb target/release/bundle/appimage/*.AppImage
