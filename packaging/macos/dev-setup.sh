#!/bin/bash
# Development tools on a Mac (Apple Silicon, for example a rented Scaleway Mac mini): Xcode's
# command line tools, Homebrew, Rust, Node 24 (as CI), uv, cliclick (clicks and keys from a
# shell, for tests), BlackHole and SwitchAudioSource (a virtual microphone, for replayed speech:
# the Mac plays a clip into BlackHole, Utter hears it), and the repository.
# Run once in Terminal on the Mac's desktop; it asks for the account password where macOS needs
# admin. Running it again only fills in what is missing.
set -euo pipefail

# Asks for the password once, unless sudo needs none (sudo -v asks even then on macOS).
sudo -n true 2>/dev/null || sudo -v

if ! xcode-select -p >/dev/null 2>&1; then
    echo "== Xcode command line tools"
    flag=/tmp/.com.apple.dt.CommandLineTools.installondemand.in-progress
    touch "$flag"
    label=$(softwareupdate -l 2>/dev/null | sed -n 's/^.*Label: \(Command Line Tools.*\)$/\1/p' | tail -1)
    sudo softwareupdate -i "$label" --verbose
    rm -f "$flag"
fi

if [ ! -x /opt/homebrew/bin/brew ]; then
    echo "== Homebrew"
    NONINTERACTIVE=1 /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
fi
eval "$(/opt/homebrew/bin/brew shellenv)"
line='eval "$(/opt/homebrew/bin/brew shellenv)"'
grep -qF "$line" ~/.zprofile 2>/dev/null || echo "$line" >> ~/.zprofile
line='export PATH="/opt/homebrew/opt/node@24/bin:$PATH"'
grep -qF "$line" ~/.zprofile || echo "$line" >> ~/.zprofile

echo "== Node, uv, cliclick, SwitchAudioSource, BlackHole"
brew install node@24 uv cliclick switchaudio-osx
brew list --cask blackhole-2ch >/dev/null 2>&1 || brew install --cask blackhole-2ch

if [ ! -x ~/.cargo/bin/rustup ]; then
    echo "== Rust"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal -c clippy,rustfmt
fi

if [ ! -d ~/utter/.git ]; then
    echo "== Repository"
    git clone https://github.com/ductai199x/utter ~/utter
fi

echo
echo "Done. macOS $(sw_vers -productVersion), $(sysctl -n machdep.cpu.brand_string)."
echo "Next, in System Settings > Privacy & Security, add /usr/libexec/sshd-keygen-wrapper to"
echo "Accessibility and Screen & System Audio Recording, so tests over SSH can see and click."
