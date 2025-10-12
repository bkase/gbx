{ pkgs, config, lib, ... }:
let
  basePkgs = with pkgs; [ jq git fd ripgrep cargo-nextest trunk ];
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

  enterShell = '' '';

  # Define tasks first so hooks can reference them
  tasks."format:workspace".exec = "cargo fmt --all";
  tasks."format:check".exec = "cargo fmt --all -- --check";
  tasks."lint:workspace".exec =
    "cargo clippy --all-targets --all-features -- -D warnings";
  tasks."build:workspace".exec = "cargo build --all-targets";
  tasks."build:wasm".exec =
    "cargo build --target wasm32-unknown-unknown -p app";
  tasks."test:workspace".exec = "cargo test --all-targets";
  tasks."web:watch".exec = "trunk watch --config web/trunk.toml";
  tasks."web:serve".exec =
    "cargo run -p dev_server -- --dist web/dist --port 8080";

  # Git hooks for code quality enforcement
  pre-commit.hooks = {
    format-check = {
      enable = true;
      name = "format:check";
      entry = "${pkgs.writeShellScript "format-check-hook" ''
        devenv tasks run format:check
      ''}";
      pass_filenames = false;
    };

    lint-workspace = {
      enable = true;
      name = "lint:workspace";
      entry = "${pkgs.writeShellScript "lint-workspace-hook" ''
        devenv tasks run lint:workspace
      ''}";
      pass_filenames = false;
    };

    test-workspace = {
      enable = true;
      name = "test:workspace";
      entry = "${pkgs.writeShellScript "test-workspace-hook" ''
        devenv tasks run test:workspace
      ''}";
      pass_filenames = false;
      stages = ["pre-commit"];
    };

    build-all = {
      enable = true;
      name = "build:all";
      entry = "${pkgs.writeShellScript "build-all-hook" ''
        set -e
        echo "Running full workspace verification..."
        devenv tasks run build:workspace
        devenv tasks run build:wasm
      ''}";
      pass_filenames = false;
      stages = ["pre-push"];
    };

    commit-msg-format = {
      enable = true;
      name = "commit-msg:format";
      entry = "${pkgs.writeShellScript "commit-msg-format-hook" ''
        exec ${config.devenv.root}/scripts/validate-commit-msg.sh "$1"
      ''}";
      pass_filenames = true;
      stages = ["commit-msg"];
    };
  };
}
