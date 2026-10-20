# Homebrew cask for Utter: brew install --cask ductai199x/utter/utter
# Generated from this template by packaging/macos/cask.sh (VERSION, SHA256, URL filled in).
cask "utter" do
  version "VERSION"
  sha256 "SHA256"

  url "URL"
  name "Utter"
  desc "Local voice dictation: hold a key, speak, and clean text appears where you type"
  homepage "https://github.com/ductai199x/utter"

  depends_on macos: ">= :sonoma"
  depends_on arch: :arm64

  app "Utter.app"

  # Utter is signed with its own certificate, not an Apple Developer ID, so Gatekeeper would
  # refuse to open a quarantined copy ("cannot be verified"), and macOS 15 and later no longer
  # offer Open from the context menu. Homebrew checked the download's checksum above.
  postflight do
    system_command "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "#{appdir}/Utter.app"]
  end

  # Quits Utter and removes its sign-in entry: with the app gone, launchd would try to start a
  # missing program at every sign-in.
  uninstall quit:   "app.utter",
            delete: "~/Library/LaunchAgents/app.utter.plist"

  # brew uninstall --zap: everything Utter wrote, its downloaded models (several GB) included.
  zap trash: [
    "~/Library/Application Support/Utter",
    "~/Library/Caches/app.utter",
    "~/Library/HTTPStorages/app.utter",
    "~/Library/Preferences/app.utter.plist",
    "~/Library/WebKit/app.utter",
  ]

  caveats <<~EOS
    Open Utter from Applications. It asks for Accessibility (to hear the dictation key and paste)
    and the microphone, then downloads its models (about 7 GB).
  EOS
end
