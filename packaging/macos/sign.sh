#!/bin/bash
# Signs Utter.app with Utter's own code signing certificate, the same one for every release.
# macOS ties a permission (Accessibility, Microphone) to the app's designated requirement; with
# an ad-hoc signature that is the exact build, so every update lost the grants while the switch
# in System Settings still showed on (measured on macOS 26.7). Signed with one certificate the
# requirement is "app.utter, signed by this certificate", the same for every build.
#
# Usage: sign.sh path/to/Utter.app
# The certificate's identity comes from UTTER_SIGNING_IDENTITY (its SHA-1) in the keychain
# UTTER_SIGNING_KEYCHAIN, unlocked with the password in UTTER_SIGNING_KEYCHAIN_PASS.
set -euo pipefail
app=${1:?usage: sign.sh path/to/Utter.app}
here=$(cd "$(dirname "$0")" && pwd)
: "${UTTER_SIGNING_IDENTITY:?the certificate SHA-1}"
: "${UTTER_SIGNING_KEYCHAIN:?the keychain holding it}"
if [ -n "${UTTER_SIGNING_KEYCHAIN_PASS:-}" ]; then
    security unlock-keychain -p "$UTTER_SIGNING_KEYCHAIN_PASS" "$UTTER_SIGNING_KEYCHAIN"
fi
codesign --force --options runtime --timestamp=none \
    --entitlements "$here/../../app/src-tauri/macos/Utter.entitlements" \
    --keychain "$UTTER_SIGNING_KEYCHAIN" -s "$UTTER_SIGNING_IDENTITY" "$app"
codesign --verify --strict "$app"
codesign -dr - "$app" 2>&1 | grep designated
