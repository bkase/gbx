{ pkgs, config, lib, ... }:
let
  basePkgs = with pkgs; [
    jq
    git
    fd
    ripgrep
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
    # Property test defaults for fast lane (bounded)
    PROPTEST_CASES = "32";
    PROPTEST_TIMEOUT = "2000"; # ms for the whole property test
    # Nicer output locally
    NEXTEST_HIDE_PROGRESS_BAR = "0";
  };

  # Native rustflags for stricter compilation
  # Set this manually in build commands to avoid breaking web builds
  env.NATIVE_RUSTFLAGS = lib.concatStringsSep " " [
    "-D" "warnings"
    "-D" "missing_docs"
  ];

  # WASM-specific rustflags for shared linear memory
  env.WASM_RUSTFLAGS = lib.concatStringsSep " " [
    "-Z" "unstable-options"
    "-C" "panic=immediate-abort"
    "-C" "target-feature=+atomics,+bulk-memory,+mutable-globals"
    "-C" "link-arg=--shared-memory"
    "-C" "link-arg=--import-memory"
    "-C" "link-arg=--export=__wasm_init_tls"
    "-C" "link-arg=--export=__wasm_init_memory"
    "-C" "link-arg=--export=__tls_size"
    "-C" "link-arg=--export=__tls_align"
    "-C" "link-arg=--export=__tls_base"
    "-C" "link-arg=--max-memory=67108864"
    "-D" "warnings"
    "-D" "missing_docs"
  ];

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
  tasks."assets:testroms".exec = ''
    set -euo pipefail

    dest="third_party/testroms/c-sp-v7.0"
    if [ -d "$dest/mooneye-test-suite" ]; then
      exit 0
    fi

    mkdir -p third_party/testroms

    tmp="$(mktemp)"
    curl -L -o "$tmp" "https://github.com/c-sp/game-boy-test-roms/releases/download/v7.0/game-boy-test-roms-v7.0.zip"

    echo "b9a9d7a1075aa35a3d07c07c34974048672d8520dca9e07a50178f5860c3832c  $tmp" | shasum -a 256 -c -

    staging="$(mktemp -d)"
    unzip -q "$tmp" -d "$staging"
    rm "$tmp"

    rm -rf "$dest"
    mkdir -p "$dest"
    find "$staging" -mindepth 1 -maxdepth 1 -print0 | while IFS= read -r -d "" entry; do
      name="$(basename "$entry")"
      mv "$entry" "$dest/$name"
    done
    rm -rf "$staging"
  '';

  tasks."format:workspace".exec = "cargo fmt --all";
  tasks."format:check".exec = "cargo fmt --all -- --check";
  tasks."lint:workspace".exec =
    ''
      devenv tasks run assets:testroms
      python3 scripts/check-layer-deps.py
      cargo clippy --all-targets --all-features -- -D warnings -D clippy::undocumented_unsafe_blocks
    '';
  tasks."build:workspace".exec = ''
    devenv tasks run assets:testroms
    export RUSTFLAGS="$NATIVE_RUSTFLAGS"
    cargo build --all-targets
  '';
  tasks."build:wasm".exec = ''
    export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="$WASM_RUSTFLAGS"
    cargo build --target wasm32-unknown-unknown -p app
  '';
  tasks."build:fabric-worker-wasm".exec = ''
    set -euo pipefail

    echo "Building fabric-worker-wasm with wasm-pack..."
    export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="$WASM_RUSTFLAGS"
    wasm-pack build --target web crates/06-apps/gbx-wasm --out-dir ../../../web/pkg --out-name fabric_worker_wasm -- -Z build-std=std,panic_abort

    echo "Verifying memory is imported..."
    wasm-tools print web/pkg/fabric_worker_wasm_bg.wasm | grep -E "(import.*memory)" | head -1
    echo "✅ Memory import successful! fabric-worker-wasm ready for shared memory."
  '';
  tasks."test:workspace".exec = ''
    devenv tasks run assets:testroms
    export RUSTFLAGS="$NATIVE_RUSTFLAGS"
    cargo test --all-targets
    cargo test -p transport-fabric --features proptest --test mailbox_tests
  '';
  tasks."test:golden".exec = ''
    devenv tasks run assets:testroms
    export RUSTFLAGS="$NATIVE_RUSTFLAGS"
    cargo test -p tests transport_schema_goldens_v1
  '';

  # Tight test discipline tasks (stable API for CI)
  tasks."test:fast".exec = ''
    devenv tasks run assets:testroms
    export RUSTFLAGS="$NATIVE_RUSTFLAGS"
    cargo nextest run --profile fast
  '';
  tasks."test:slow".exec = ''
    devenv tasks run assets:testroms
    export RUSTFLAGS="$NATIVE_RUSTFLAGS"
    cargo nextest run --profile slow --run-ignored ignored-only --features loom
  '';
  tasks."test:wasm-smoke".exec = ''
    set -euo pipefail

    repo_name="$(basename "$PWD")"
    hash_hex="$(printf "%s" "$repo_name" | sha256sum | cut -c1-8)"
    hash_dec=$((16#$hash_hex))
    wasm_port=$((8000 + hash_dec % 999))
    echo "Using wasm smoke test port: ''${wasm_port}"

    echo "Building fabric-worker-wasm with wasm-pack..."
    devenv tasks run build:fabric-worker-wasm

    echo "Copying artifacts to tests/wasm/pkg..."
    rm -rf tests/wasm/pkg
    mkdir -p tests/wasm/pkg
    cp web/pkg/fabric_worker_wasm.js tests/wasm/pkg/
    cp web/pkg/fabric_worker_wasm_bg.wasm tests/wasm/pkg/
    cp web/pkg/fabric_worker_wasm_bg.wasm.d.ts tests/wasm/pkg/ 2>/dev/null || true
    cp web/pkg/fabric_worker_wasm.d.ts tests/wasm/pkg/ 2>/dev/null || true
    cp web/worker.js tests/wasm/pkg/

    npm install --silent >/dev/null
    bash scripts/run-browser-test.sh tests/wasm "''${wasm_port}" tests/wasm_browser_test.js
  '';

  tasks."test:wasm-light".exec = ''
    set -euo pipefail

    repo_name="$(basename "$PWD")"
    hash_hex="$(printf "%s" "$repo_name" | sha256sum | cut -c1-8)"
    hash_dec=$((16#$hash_hex))
    wasm_port=$((8000 + hash_dec % 999))
    echo "Using wasm light test port: ''${wasm_port}"

    npm install --silent >/dev/null
    bash scripts/run-browser-test.sh tests/wasm "''${wasm_port}" tests/wasm_browser_test.js
  '';

  tasks."test:demo".exec = ''
    set -euo pipefail

    repo_name="$(basename "$PWD")"
    hash_hex="$(printf "%s" "$repo_name" | sha256sum | cut -c1-8)"
    hash_dec=$((16#$hash_hex))
    base_port=$((8000 + hash_dec % 999))
    demo_port=$((base_port + 1))
    echo "Using demo test ports: wasm=''${base_port} demo=''${demo_port}"

    echo "Building UI demo with wasm-pack..."
    devenv tasks run build:fabric-worker-wasm

    npm install --silent >/dev/null
    bash scripts/run-browser-test.sh web "''${demo_port}" tests/demo_browser_test.js
  '';

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
