class NvmRust < Formula
  desc "A blazing-fast Node version manager written in Rust"
  homepage "https://github.com/mose-x/nvm-rust"
  version "2.0.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-macos-x64.tar.gz"
      sha256 "8af3dc4a4d9391cfb849dea2f78b64850b538a085f79e41906638c22b13c3cb3"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-macos-arm64.tar.gz"
      sha256 "f8da8705f1ccb21e14bb6f0f9168df654786a50909798a8e02f44bde173cf756"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-linux-x64.tar.gz"
      sha256 "a10055b2940600c406a28897b63fdbd56644c5a86f26fa6bb6dabf02a35154a7"
    elsif Hardware::CPU.arm?
      url "https://github.com/mose-x/nvm-rust/releases/download/v2.0.0/nvm-2.0.0-linux-arm64.tar.gz"
      sha256 "7a17351b58937a5801934250c5b85bd056407893d0939e194bfe0ec90b6d9465"
    end
  end

  def install
    bin.install "nvm"
  end

  test do
    assert_match "nvm", shell_output("#{bin}/nvm --version")
  end
end
