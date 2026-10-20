# macOS packaging

Utter ships on macOS through Homebrew, from this repository as its own tap (no second repo):

```bash
brew tap ductai199x/utter https://github.com/ductai199x/utter
brew trust --cask ductai199x/utter/utter
brew install --cask utter
```

`Casks/utter.rb` at the repository root is the cask Homebrew reads. It always names the latest
release; nobody edits it by hand.

## Signing

Utter is signed with its own certificate ("Utter Homebrew Signing", SHA-1
`E411284D187F5D405F62FCB184A7985F7DB275C2`, valid to 2036), not an Apple Developer ID. macOS ties
the Accessibility and Microphone permissions to the signing certificate, so they survive every
update; an ad-hoc signature lost them at each update while System Settings still showed the
switch on (measured on macOS 26.7, 2026-10-19). Every release must be signed with this same
certificate. Its key is kept outside the repository, in the maintainer's backups
(`utter.p12` and the PEM files); losing it means every user grants the
permissions again once.

To sign on a Mac, import `utter.p12` into a keychain of its own, then `sign.sh` (it reads the
identity and keychain from `UTTER_SIGNING_IDENTITY`, `UTTER_SIGNING_KEYCHAIN` and
`UTTER_SIGNING_KEYCHAIN_PASS`).

## A release

On an Apple Silicon Mac, from the repository root, after the version bump:

```bash
cd app && npm ci && npm run build && npx tauri build --bundles app && cd ..
packaging/macos/sign.sh target/release/bundle/macos/Utter.app
packaging/macos/cask.sh target/release/bundle/macos/Utter.app VERSION dist
```

Then attach `dist/Utter-VERSION-macos-arm64.zip` to the GitHub release `vVERSION` and commit
`Casks/utter.rb`, after the release is published (the cask's URL must work when it lands).

To test a cask before publishing: `cask.sh ... VERSION dist file://$PWD/dist`, copy the generated
`Casks/utter.rb` into a local tap (`brew tap-new --no-git you/local`), and
`brew install --cask you/local/utter`. Do not commit that `Casks/utter.rb` (its URL is a file).

How to rent, set up, drive and test on a Mac, and every macOS lesson so far: [docs/development.md](https://github.com/ductai199x/utter/blob/dev/docs/development.md).

`dev-setup.sh` prepares a Mac for development (Xcode tools, Homebrew, Rust, Node, BlackHole as
a virtual microphone for replayed speech).
