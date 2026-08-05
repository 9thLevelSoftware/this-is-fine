# Homebrew formula stub for This Is Fine (`tif`).
#
# Not yet published to homebrew-core. Local tap usage:
#   brew install --build-from-source ./dist/homebrew/this-is-fine.rb
# Or after a tap is published:
#   brew install 9thLevelSoftware/tap/this-is-fine
#
# Keep version/sha256 in sync with GitHub Releases (see scripts/install.sh).

class ThisIsFine < Formula
  desc "Local-first containment and simplification governor for coding agents"
  homepage "https://github.com/9thLevelSoftware/this-is-fine"
  version "0.1.0"
  license "MIT OR Apache-2.0"

  on_macos do
    on_arm do
      # url "https://github.com/9thLevelSoftware/this-is-fine/releases/download/v#{version}/tif-aarch64-apple-darwin.tar.gz"
      # sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      # url "https://github.com/9thLevelSoftware/this-is-fine/releases/download/v#{version}/tif-x86_64-apple-darwin.tar.gz"
      # sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  on_linux do
    on_arm do
      # url "https://github.com/9thLevelSoftware/this-is-fine/releases/download/v#{version}/tif-aarch64-unknown-linux-gnu.tar.gz"
      # sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
    on_intel do
      # url "https://github.com/9thLevelSoftware/this-is-fine/releases/download/v#{version}/tif-x86_64-unknown-linux-gnu.tar.gz"
      # sha256 "REPLACE_WITH_RELEASE_SHA256"
    end
  end

  # Source fallback until release assets are filled in above.
  url "https://github.com/9thLevelSoftware/this-is-fine.git", tag: "v0.1.0", revision: "REPLACE_WITH_TAG_SHA"
  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--root", prefix, "--path", "crates/tif"
  end

  test do
    assert_match "tif", shell_output("#{bin}/tif --version")
  end
end
