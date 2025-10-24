{ pkgs, config, lib, ... }:
let
  basePkgs = with pkgs; [
    jq
    git
    fd
    ripgrep
    cmake
    pkg-config
    llvmPackages_18.clang
    llvmPackages_18.libclang
    cargo-nextest
    wasm-tools
    wabt
    chromedriver
    python3
    rustup
    nodejs
    curl
    unzip
  ];

in {
  packages = basePkgs;

  languages.rust = {
    enable = true;
    channel = "nightly";
    version = "2025-10-16";  # Known to work with --import-memory + --shared-memory
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" "rust-src" ];
    targets = [ "wasm32-unknown-unknown" ];
  };

  env = {
    CARGO_TERM_COLOR = "always";
    RUSTUP_TOOLCHAIN = "nightly-2025-10-16";
    # Property test defaults for fast lane (bounded)
    PROPTEST_CASES = "32";
    PROPTEST_TIMEOUT = "2000"; # ms for the whole property test
    # Nicer output locally
    NEXTEST_HIDE_PROGRESS_BAR = "0";
  };

  # Single-source rustflags (shared with cloud runners)
  env.NATIVE_RUSTFLAGS = builtins.readFile ./scripts/env/NATIVE_RUSTFLAGS;
  env.WASM_RUSTFLAGS = builtins.readFile ./scripts/env/WASM_RUSTFLAGS;

  enterShell = ''
    # Ensure wasm-pack is installed via cargo
    if ! command -v wasm-pack &> /dev/null; then
      echo "Installing wasm-pack via cargo..."
      cargo install wasm-pack
    fi
  '';

  # Scripts for commit message validation and other utilities
  scripts.validate-commit-msg.exec = ''
    # Validates commit message format:
    # - Subject line: max 50 characters, imperative present-tense
    # - Blank line between subject and body
    # - Body lines: max 72 characters

    set -euo pipefail

    commit_msg=$(cat "$1")
    first_line=$(echo "$commit_msg" | head -n1)
    subject_length=''${#first_line}

    # Check subject line length (max 50 characters)
    if [ "$subject_length" -gt 50 ]; then
      echo "Error: Commit subject line is too long ($subject_length characters, max 50)"
      echo "Keep the first line concise and add details in the body after a blank line."
      echo ""
      echo "Your subject: $first_line"
      exit 1
    fi

    # Check for past tense (should be imperative present-tense)
    if echo "$first_line" | grep -qE '^(Added|Fixed|Updated|Removed|Changed|Deleted|Created|Refactored)'; then
      echo "Error: Commit message should use imperative present-tense:"
      echo "  Use 'Add' not 'Added'"
      echo "  Use 'Fix' not 'Fixed'"
      echo "  Use 'Update' not 'Updated'"
      echo ""
      echo "Your subject: $first_line"
      exit 1
    fi

    # Ensure it starts with a capital letter and a verb
    if ! echo "$first_line" | grep -qE '^[A-Z][a-z]+ '; then
      echo "Error: Commit subject should start with an imperative verb (e.g., 'Add', 'Fix', 'Update')"
      echo "Your subject: $first_line"
      exit 1
    fi

    # Check for blank line between subject and body if body exists
    line_count=$(echo "$commit_msg" | wc -l | tr -d ' ')
    if [ "$line_count" -gt 1 ]; then
      second_line=$(echo "$commit_msg" | sed -n '2p')
      if [ -n "$second_line" ]; then
        echo "Error: Commit message must have a blank line between subject and body"
        echo ""
        echo "Format:"
        echo "  Subject line (max 50 chars)"
        echo "  "
        echo "  Detailed explanation in the body (wrap at 72 chars)..."
        exit 1
      fi
    fi

    # Check body line lengths (max 72 characters), skipping subject and blank line
    if [ "$line_count" -gt 2 ]; then
      body_lines=$(echo "$commit_msg" | tail -n +3)
      line_num=3
      while IFS= read -r line; do
        line_length=''${#line}
        if [ "$line_length" -gt 72 ]; then
          echo "Error: Body line $line_num is too long ($line_length characters, max 72)"
          echo "Wrap body text at 72 characters for readability."
          echo ""
          echo "Line $line_num: $line"
          exit 1
        fi
        line_num=$((line_num + 1))
      done <<< "$body_lines"
    fi
  '';

  # Define tasks first so hooks can reference them
  tasks."assets:testroms".exec = "bash scripts/tasks/run.sh assets:testroms";
  tasks."format:workspace".exec = "bash scripts/tasks/run.sh format:workspace";
  tasks."format:check".exec = "bash scripts/tasks/run.sh format:check";
  tasks."lint:workspace".exec = "bash scripts/tasks/run.sh lint:workspace";
  tasks."build:workspace".exec = "bash scripts/tasks/run.sh build:workspace";
  tasks."build:wasm".exec = "bash scripts/tasks/run.sh build:wasm";
  tasks."build:fabric-worker-wasm".exec = "bash scripts/tasks/run.sh build:fabric-worker-wasm";
  tasks."test:workspace".exec = "bash scripts/tasks/run.sh test:workspace";
  tasks."test:golden".exec = "bash scripts/tasks/run.sh test:golden";
  tasks."test:fast".exec = "bash scripts/tasks/run.sh test:fast";
  tasks."test:slow".exec = "bash scripts/tasks/run.sh test:slow";
  tasks."test:wasm-smoke".exec = "bash scripts/tasks/run.sh test:wasm-smoke";
  tasks."test:wasm-light".exec = "bash scripts/tasks/run.sh test:wasm-light";
  tasks."test:demo".exec = "bash scripts/tasks/run.sh test:demo";

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
        set -e
        devenv tasks run test:golden
        devenv tasks run test:workspace
        devenv tasks run test:wasm-smoke
        devenv tasks run test:demo
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
        exec validate-commit-msg "$1"
      ''}";
      pass_filenames = true;
      stages = ["commit-msg"];
    };
  };
}
