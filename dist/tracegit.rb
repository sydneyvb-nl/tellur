# Homebrew formula for TraceGit.
#
# Builds from source so it works on every supported architecture without
# maintaining per-platform bottle checksums. Install with:
#
#   brew install --build-from-source ./dist/tracegit.rb
#
# or, once tapped:
#
#   brew install sydneyvb-nl/tap/tracegit
class Tracegit < Formula
  desc "AI code provenance — line-level attribution, PR risk reports, policy-as-code"
  homepage "https://github.com/sydneyvb-nl/TraceGit"
  url "https://github.com/sydneyvb-nl/TraceGit/archive/refs/tags/v0.1.0.tar.gz"
  # Replace with the real tarball SHA-256 at release time:
  #   curl -sL <url> | shasum -a 256
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "Apache-2.0"
  head "https://github.com/sydneyvb-nl/TraceGit.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", "--locked", "--root", prefix, "--path", "crates/cli"
  end

  test do
    assert_match "tracegit", shell_output("#{bin}/tracegit --version")
    system bin/"tracegit", "--help"
  end
end
