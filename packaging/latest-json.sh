#!/bin/bash
# Signs a release's update files with Utter's updater key and writes latest.json, the file the
# app asks for (app/src-tauri/src/update.rs; plugins.updater in tauri.conf.json). Attach every
# file this lists, and latest.json itself, to the GitHub release vVERSION.
#
# Usage: packaging/latest-json.sh VERSION DIR [URL_BASE]
#   DIR holds Utter_VERSION_x64-setup.exe, Utter_VERSION_amd64.AppImage, Utter_VERSION_amd64.deb
#   and Utter-VERSION-macos-arm64.app.tar.gz (packaging/macos/cask.sh makes it); the first line
#   of DIR/notes.md, if there, goes in as the update's description.
#   URL_BASE defaults to the GitHub release; a folder served over http tests an update.
# The key: UTTER_UPDATER_KEY (a file) and UTTER_UPDATER_KEY_PASS (its password file), by default
# ~/.utter-signing/updater.key and updater.pass.
#
# The updater asks for "{os}-{arch}-{installer}" first, then "{os}-{arch}". Each Linux install
# gets only its own kind (an AppImage is never offered the .deb, nor the .deb an AppImage);
# Windows and macOS have one each and get both names.
set -euo pipefail
version=${1:?usage: latest-json.sh VERSION DIR [URL_BASE]}
dir=${2:?dir}
base=${3:-https://github.com/ductai199x/utter/releases/download/v$version}
key=${UTTER_UPDATER_KEY:-$HOME/.utter-signing/updater.key}
pass=$(cat "${UTTER_UPDATER_KEY_PASS:-$HOME/.utter-signing/updater.pass}")
here=$(cd "$(dirname "$0")" && pwd)

entries=""
add() { # FILE PLATFORM...
    local file=$1
    shift
    [ -f "$dir/$file" ] || { echo "missing $dir/$file" >&2; exit 1; }
    (cd "$here/../app" && npx tauri signer sign -f "$key" -p "$pass" "$dir/$file" >/dev/null)
    local sig
    sig=$(cat "$dir/$file.sig")
    for platform in "$@"; do
        entries+="$([ -n "$entries" ] && echo ,)\"$platform\":{\"signature\":\"$sig\",\"url\":\"$base/$file\"}"
    done
}
add "Utter_${version}_x64-setup.exe" windows-x86_64-nsis windows-x86_64
add "Utter_${version}_amd64.AppImage" linux-x86_64-appimage
add "Utter_${version}_amd64.deb" linux-x86_64-deb
add "Utter-${version}-macos-arm64.app.tar.gz" darwin-aarch64-app darwin-aarch64

notes=""
[ -f "$dir/notes.md" ] && notes=$(sed -n '1p' "$dir/notes.md" | sed 's/\\/\\\\/g; s/"/\\"/g')
printf '{"version":"%s","notes":"%s","pub_date":"%s","platforms":{%s}}\n' \
    "$version" "$notes" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$entries" > "$dir/latest.json"
echo "$dir/latest.json"
