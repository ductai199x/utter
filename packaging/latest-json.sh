#!/bin/bash
# Signs a release's update files with Utter's updater key and writes latest.json, the file the
# app asks for (app/src-tauri/src/update.rs; plugins.updater in tauri.conf.json). Attach every
# file this lists, and latest.json itself, to the GitHub release vVERSION.
#
# Usage: packaging/latest-json.sh VERSION DIR [URL_BASE]
#   DIR holds Utter_VERSION_x64-setup.exe, Utter_VERSION_amd64.AppImage and
#   Utter-VERSION-macos-arm64.app.tar.gz (packaging/macos/cask.sh makes it); the release notes,
#   if DIR/notes.md exists, go in as the update's description.
#   URL_BASE defaults to the GitHub release; a local folder served over http tests an update.
# The key: UTTER_UPDATER_KEY (a file) and UTTER_UPDATER_KEY_PASS (its password file), by default
# ~/.utter-signing/updater.key and updater.pass.
set -euo pipefail
version=${1:?usage: latest-json.sh VERSION DIR [URL_BASE]}
dir=${2:?dir}
base=${3:-https://github.com/ductai199x/utter/releases/download/v$version}
key=${UTTER_UPDATER_KEY:-$HOME/.utter-signing/updater.key}
pass=$(cat "${UTTER_UPDATER_KEY_PASS:-$HOME/.utter-signing/updater.pass}")
here=$(cd "$(dirname "$0")" && pwd)

declare -A files=(
    [windows-x86_64]="Utter_${version}_x64-setup.exe"
    [linux-x86_64]="Utter_${version}_amd64.AppImage"
    [darwin-aarch64]="Utter-${version}-macos-arm64.app.tar.gz"
)
platforms=""
for platform in windows-x86_64 linux-x86_64 darwin-aarch64; do
    file="$dir/${files[$platform]}"
    [ -f "$file" ] || { echo "missing $file" >&2; exit 1; }
    (cd "$here/../app" && npx tauri signer sign -f "$key" -p "$pass" "$file" >/dev/null)
    sig=$(cat "$file.sig")
    platforms+="$([ -n "$platforms" ] && echo ,)\"$platform\":{\"signature\":\"$sig\",\"url\":\"$base/${files[$platform]}\"}"
done
notes=""
[ -f "$dir/notes.md" ] && notes=$(sed -n '1p' "$dir/notes.md" | sed 's/\\/\\\\/g; s/"/\\"/g')
printf '{"version":"%s","notes":"%s","pub_date":"%s","platforms":{%s}}\n' \
    "$version" "$notes" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$platforms" > "$dir/latest.json"
echo "$dir/latest.json"
