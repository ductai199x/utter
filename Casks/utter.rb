# Homebrew cask for Utter. This repository is its own tap:
#   brew tap ductai199x/utter https://github.com/ductai199x/utter && brew install --cask utter
# The template: packaging/macos/cask.sh fills in VERSION, SHA256 and URL into Casks/utter.rb.
cask "utter" do
  version "0.1.9"
  sha256 "10081192f1aba26b62962eac2a46f86449654c66518526db1d97b5f62bd5a71b"

  url "https://github.com/ductai199x/utter/releases/download/v0.1.9/Utter-0.1.9-macos-arm64.zip"
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
