{ pkgs, config, lib, ... }:
let
  basePkgs = with pkgs; [
    jq
    git
    fd
    ripgrep
    cargo-nextest
    trunk
    wasm-pack
    wasm-tools
    wabt
    chromedriver
    python3
    rustup
  ];

  # Shared WASM build flags for atomics and shared memory
  wasmSharedMemoryFlags = "-Z unstable-options -C panic=abort -C target-feature=+atomics,+bulk-memory,+mutable-globals -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__wasm_init_memory -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base -C link-arg=--max-memory=1073741824";
in {
  packages = basePkgs;

  languages.rust = {
    enable = true;
    channel = "nightly";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" "rust-src" ];
    targets = [ "wasm32-unknown-unknown" ];
    # Strict compilation: warnings as errors, require docs on public items
    # For native builds, use strict warnings
    rustflags = "-D warnings -D missing_docs";
  };

  env = {
    CARGO_TERM_COLOR = "always";
    # Property test defaults for fast lane (bounded)
    PROPTEST_CASES = "32";
    PROPTEST_TIMEOUT = "2000"; # ms for the whole property test
    # Nicer output locally
    NEXTEST_HIDE_PROGRESS_BAR = "0";
    # WASM build flags: shared memory with atomics
    # These apply to wasm32-unknown-unknown target builds
    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS = wasmSharedMemoryFlags;
    WASM_RUSTFLAGS = wasmSharedMemoryFlags;
  };

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
  tasks."format:workspace".exec = "cargo fmt --all";
  tasks."format:check".exec = "cargo fmt --all -- --check";
  tasks."lint:workspace".exec =
    "cargo clippy --all-targets --all-features -- -D warnings -D clippy::undocumented_unsafe_blocks";
  tasks."build:workspace".exec = "cargo build --all-targets";
  tasks."build:wasm".exec =
    "cargo build --target wasm32-unknown-unknown -p app";
  tasks."build:transport-worker".exec = ''
    set -euo pipefail

    echo "Building transport-worker..."

    # Build with cargo using shared memory support
    cargo build -p transport-worker \
      --target wasm32-unknown-unknown \
      --release \
      -Z build-std=std,panic_abort

    mkdir -p web/pkg

    # Use wasm-bindgen-cli to generate JS bindings
    # Try to find wasm-bindgen in cargo bin, otherwise install it
    WASM_BINDGEN="$HOME/.cargo/bin/wasm-bindgen"
    if [ ! -f "$WASM_BINDGEN" ]; then
      echo "Installing wasm-bindgen-cli..."
      cargo install wasm-bindgen-cli --version 0.2.104
    fi

    # Generate bindings
    $WASM_BINDGEN \
      target/wasm32-unknown-unknown/release/transport_worker.wasm \
      --out-dir web/pkg \
      --out-name transport_worker \
      --target web

    # Verify the wasm module was generated
    output="web/pkg/transport_worker_bg.wasm"

    if [ ! -f "$output" ]; then
      echo "expected wasm-bindgen artifact at $output" >&2
      exit 1
    fi

    # Note: wasm-bindgen creates its own memory by default, but we use
    # wasm_bindgen::memory() to share memory between main and worker
    echo "transport-worker built successfully"
    wasm-objdump -x -j import "$output" | head -20
  '';
  tasks."test:workspace".exec = "cargo test --all-targets";
  tasks."test:golden".exec =
    "cargo test -p tests transport_schema_goldens_v1";

  # Tight test discipline tasks (stable API for CI)
  tasks."test:fast".exec = "cargo nextest run --profile fast";
  tasks."test:slow".exec =
    "cargo nextest run --profile slow --run-ignored ignored-only --features loom";
  tasks."test:wasm-smoke".exec = ''
    set -euo pipefail

    # Build the transport-worker using wasm-pack
    echo "Building transport-worker with wasm-pack..."
    devenv tasks run build:transport-worker

    echo "Building test WASM..."
    rm -rf tests/wasm/pkg
    mkdir -p tests/wasm/pkg

    # Use RUSTFLAGS for wasm-pack (it doesn't respect CARGO_TARGET_*_RUSTFLAGS)
    RUSTFLAGS="${wasmSharedMemoryFlags}" \
      wasm-pack build crates/tests --target web --out-dir ../../tests/wasm/pkg -- -Z build-std=std,panic_abort

    npm install --silent >/dev/null

    export WASM_TEST_PORT=4510
    node tests/wasm_server.js &
    SERVER_PID=$!
    trap "kill $SERVER_PID 2>/dev/null || true" EXIT

    sleep 3

    node tests/wasm_browser_test.js
  '';

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
        set -e
        devenv tasks run test:golden
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
        exec validate-commit-msg "$1"
      ''}";
      pass_filenames = true;
      stages = ["commit-msg"];
    };
  };
}
