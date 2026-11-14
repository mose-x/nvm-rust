class NvmRust < Formula
  desc "A blazing-fast Node version manager written in Rust"
  homepage "https://github.com/mose-x/nvm-rust"
  version "2.4.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.4.0/nvm-2.4.0-macos-x64.tar.gz"
      sha256 "9f2d4b57727429449d6b96afd1ab18ce81aabcb1724fbe9632a20bad5c45863f"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.4.0/nvm-2.4.0-macos-arm64.tar.gz"
      sha256 "109d5ecf4a275fe9eb034f381b7110ae093f8f2a4090b93edf525718c3aecd23"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.4.0/nvm-2.4.0-linux-x64.tar.gz"
      sha256 "67b85f6ecbd5fe37abf93b3eeb5d2475eb2a6682d86364f448d6dd15bf9c8509"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.4.0/nvm-2.4.0-linux-arm64.tar.gz"
      sha256 "abaaeb509366db34a22b6a335c0c9dcf12f17ab69248de6c8a0d1659865114c0"
    end
  end

  def install
    bin.install "nvm"
  end

  test do
    assert_match "nvm", shell_output("#{bin}/nvm --version")
  end
end
