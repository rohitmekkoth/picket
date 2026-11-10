# Template for the cask in the owner's tap; the release workflow fills in the version
# and checksum. Install with: brew install --cask rohitmekkoth/tap/picket
cask "picket" do
  version "@VERSION@"
  sha256 "@SHA256@"

  url "https://github.com/rohitmekkoth/picket/releases/download/v#{version}/Picket_#{version}_aarch64.dmg"
  name "Picket"
  desc "Independent, local record of what your AI agent does in Gmail and Calendar"
  homepage "https://github.com/rohitmekkoth/picket"

  depends_on arch: :arm64
  depends_on macos: :ventura

  app "Picket.app"

  uninstall launchctl: "Picket",
            quit:      "dev.picket.desktop"

  zap trash: [
    "~/Library/Application Support/dev.picket.desktop",
    "~/Library/LaunchAgents/Picket.plist",
  ]

  caveats <<~EOS
    Picket is signed with its own certificate, not an Apple Developer ID, so macOS
    blocks the first launch. Open Picket once, then choose Open Anyway in
    System Settings > Privacy & Security.

    To remove your data and Google sign-in too, use Settings > Uninstall and wipe
    in Picket before uninstalling.
  EOS
end
