class NvmRust < Formula
  desc "A blazing-fast Node version manager written in Rust"
  homepage "https://github.com/mose-x/nvm-rust"
  version "2.4.0"
  license "MIT"

  depends_on "rust" => :build

  stable do
    url "https://github.com/mose-x/nvm-rust/archive/refs/tags/v2.4.0.tar.gz"
    sha256 "f97a07d8105959b2590d344bd2e1eb1d5e153b01dc8ee3c8ca39dc88fad855f0"
  end

  head do
    url "https://github.com/mose-x/nvm-rust.git", branch: "main"
  end

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      Before `brew uninstall nvm-rust`, run:
        nvm uninstall --self
      to clean up shell configuration (PATH entries in ~/.zshrc).

      For full removal (nvm + all Node versions):
        nvm uninstall --all
    EOS
  end

  test do
    assert_match "nvm", shell_output("#{bin}/nvm --version")
  end
end
