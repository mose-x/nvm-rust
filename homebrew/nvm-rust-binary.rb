class NvmRust < Formula
  desc "A blazing-fast Node version manager written in Rust"
  homepage "https://github.com/mose-x/nvm-rust"
  version "2.0.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-macos-x64.tar.gz"
      sha256 "TO_BE_FILLED"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-macos-arm64.tar.gz"
      sha256 "TO_BE_FILLED"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-linux-x64.tar.gz"
      sha256 "TO_BE_FILLED"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-linux-arm64.tar.gz"
      sha256 "TO_BE_FILLED"
    end
  end

  def install
    bin.install "nvm"
  end

  test do
    assert_match "nvm", shell_output("#{bin}/nvm --version")
  end
end
