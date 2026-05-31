# Homebrew formula for Tellur.
#
# Builds from source so it works on every supported architecture without
# maintaining per-platform bottle checksums. Install with:
#
#   brew install --build-from-source ./dist/tellur.rb
#
# or, once tapped:
#
#   brew install sydneyvb-nl/tap/tellur
class Tellur < Formula
  desc "AI code provenance — line-level attribution, PR risk reports, policy-as-code"
  homepage "https://github.com/sydneyvb-nl/tellur"
  url "https://github.com/sydneyvb-nl/tellur/archive/refs/tags/v0.1.0.tar.gz"
  # Replace with the real tarball SHA-256 at release time:
  #   curl -sL <url> | shasum -a 256
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "Apache-2.0"
  head "https://github.com/sydneyvb-nl/tellur.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--root", prefix, "--path", "crates/cli"
  end

  test do
    assert_match "tellur", shell_output("#{bin}/tellur --version")
    system bin/"tellur", "--help"
  end
end
