{ pkgs, config, lib, ... }:
let
  basePkgs = with pkgs; [ jq git fd ripgrep cargo-nextest ];
in {
  packages = basePkgs;

  languages.rust = {
    enable = true;
    channel = "stable";
    version = "latest";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
    targets = [ "wasm32-unknown-unknown" ];
  };

  env = {
    CARGO_TERM_COLOR = "always";
  };

  enterShell = ''
    set -euo pipefail
    echo "rustc $(rustc --version)"
    echo "cargo $(cargo --version)"
  '';

  git-hooks.enable = false;

  tasks."format:workspace".exec = "cargo fmt --all";
  tasks."lint:workspace".exec =
    "cargo clippy --all-targets --all-features -- -D warnings";
  tasks."build:workspace".exec = "cargo build --all-targets";
  tasks."test:workspace".exec = "cargo test --all-targets";
}
