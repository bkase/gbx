{ pkgs, config, lib, ... }:
let
  basePkgs = with pkgs; [ jq git fd ripgrep cargo-nextest trunk wasm-pack chromedriver ];
in {
  packages = basePkgs;

  languages.rust = {
    enable = true;
    channel = "stable";
    version = "1.90.0";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
    targets = [ "wasm32-unknown-unknown" ];
    # Strict compilation: warnings as errors, require docs on public items
    rustflags = "-D warnings -D missing_docs";
  };

  env = {
    CARGO_TERM_COLOR = "always";
    # Property test defaults for fast lane (bounded)
    PROPTEST_CASES = "32";
    PROPTEST_TIMEOUT = "2000"; # ms for the whole property test
    # Nicer output locally
    NEXTEST_HIDE_PROGRESS_BAR = "0";
  };

  enterShell = '' '';

  # Define tasks first so hooks can reference them
  tasks."format:workspace".exec = "cargo fmt --all";
  tasks."format:check".exec = "cargo fmt --all -- --check";
  tasks."lint:workspace".exec =
    "cargo clippy --all-targets --all-features -- -D warnings -D clippy::undocumented_unsafe_blocks";
  tasks."build:workspace".exec = "cargo build --all-targets";
  tasks."build:wasm".exec =
    "cargo build --target wasm32-unknown-unknown -p app";
  tasks."test:workspace".exec = "cargo test --all-targets";

  # Tight test discipline tasks (stable API for CI)
  tasks."test:fast".exec = "cargo nextest run --profile fast";
  tasks."test:slow".exec =
    "cargo nextest run --profile slow --run-ignored ignored-only --features loom";
  tasks."test:wasm-smoke".exec =
    "wasm-pack test --headless --chrome --chromedriver=$(which chromedriver) crates/tests";

  tasks."web:watch".exec = "trunk watch --config web/trunk.toml";
  tasks."web:serve".exec =
    "cargo run -p dev_server -- --dist web/dist --port 8080";

  # Git hooks for code quality enforcement
  git-hooks.hooks = {
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
