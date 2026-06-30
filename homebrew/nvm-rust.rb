class NvmRust < Formula
  desc "A blazing-fast Node version manager written in Rust"
  homepage "https://github.com/mose-x/nvm-rust"
  version "0.1.0"
  license "MIT"

  depends_on "rust" => :build

  stable do
    url "https://github.com/mose-x/nvm-rust/archive/refs/tags/v0.1.0.tar.gz"
    sha256 "TO_BE_FILLED"
  end

  head do
    url "https://github.com/mose-x/nvm-rust.git", branch: "main"
  end

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "nvm", shell_output("#{bin}/nvm --version")
  end
end
