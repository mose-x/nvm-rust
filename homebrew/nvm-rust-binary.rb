class NvmRust < Formula
  desc "A blazing-fast Node version manager written in Rust"
  homepage "https://github.com/mose-x/nvm-rust"
  version "0.1.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v0.1.0/nvm-x86_64-apple-darwin.tar.gz"
      sha256 "TO_BE_FILLED"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v0.1.0/nvm-aarch64-apple-darwin.tar.gz"
      sha256 "TO_BE_FILLED"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v0.1.0/nvm-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "TO_BE_FILLED"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v0.1.0/nvm-aarch64-unknown-linux-gnu.tar.gz"
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
