#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

: "${PATH:="$PATH"}"
case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) ;;
  *) PATH="$PATH:$HOME/.cargo/bin" ;;
esac
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) PATH="$PATH:$HOME/.local/bin" ;;
esac
export PATH
: "${NATIVE_RUSTFLAGS:="$(cat scripts/env/NATIVE_RUSTFLAGS)"}"
: "${CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS:="$(cat scripts/env/WASM_RUSTFLAGS)"}"

note() { printf "\033[1;36m[tasks]\033[0m %s\n" "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

RUST_TOOLCHAIN_FILE="scripts/env/RUST_TOOLCHAIN"
if [ -f "$RUST_TOOLCHAIN_FILE" ]; then
  RUST_TOOLCHAIN_SPEC="$(cat "$RUST_TOOLCHAIN_FILE")"
else
  RUST_TOOLCHAIN_SPEC=""
fi

with_rustup_toolchain() {
  local cargo_bin
  cargo_bin="$(command -v cargo || true)"

  if [ -n "${DEVENV_PROFILE:-}" ] || [[ "${cargo_bin}" == /nix/store/* ]]; then
    "$@"
    return
  fi

  if have rustup && [ -n "$RUST_TOOLCHAIN_SPEC" ]; then
    RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN_SPEC" "$@"
  else
    "$@"
  fi
}

assets_testroms() {
  set -euo pipefail
  if [ "${SKIP_TESTROMS:-0}" = "1" ] || [ -n "${GBX_SKIP_TESTROMS:-}" ]; then
    note "Skipping test ROM download (SKIP_TESTROMS/GBX_SKIP_TESTROMS set)"
    return 0
  fi
  local dest="third_party/testroms/c-sp-v7.0"
  if [ -d "$dest/mooneye-test-suite" ]; then
    return 0
  fi

  mkdir -p third_party/testroms

  local tmp; tmp="$(mktemp)"
  curl -L -o "$tmp" "https://github.com/c-sp/game-boy-test-roms/releases/download/v7.0/game-boy-test-roms-v7.0.zip"

  if have sha256sum; then
    echo "b9a9d7a1075aa35a3d07c07c34974048672d8520dca9e07a50178f5860c3832c  $tmp" | sha256sum --check -
  else
    echo "b9a9d7a1075aa35a3d07c07c34974048672d8520dca9e07a50178f5860c3832c  $tmp" | shasum -a 256 -c -
  fi

  local staging; staging="$(mktemp -d)"
  unzip -q "$tmp" -d "$staging"
  rm "$tmp"

  rm -rf "$dest"
  mkdir -p "$dest"
  find "$staging" -mindepth 1 -maxdepth 1 -print0 | while IFS= read -r -d '' entry; do
    mv "$entry" "$dest/$(basename "$entry")"
  done
  rm -rf "$staging"
}

calc_port() {
  local repo_name hash_hex hash_dec base
  repo_name="$(basename "$PWD")"
  if have sha256sum; then
    hash_hex="$(printf "%s" "$repo_name" | sha256sum | cut -c1-8)"
  else
    hash_hex="$(printf "%s" "$repo_name" | shasum -a 256 | cut -c1-8)"
  fi
  hash_dec=$((16#$hash_hex))
  base=$((8000 + hash_dec % 999))
  echo "$base"
}

format_workspace() { cargo fmt --all; }
format_check()     { cargo fmt --all -- --check; }

lint_workspace() {
  assets_testroms
  python3 scripts/check-layer-deps.py
  cargo clippy --all-targets --all-features -- -D warnings -D clippy::undocumented_unsafe_blocks
}

build_workspace() {
  assets_testroms
  export RUSTFLAGS="$NATIVE_RUSTFLAGS"
  cargo build --all-targets
}

build_wasm_app() {
  assets_testroms
  export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="$CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS"
  with_rustup_toolchain cargo build --target wasm32-unknown-unknown -p app
}

build_fabric_worker_wasm() {
  assets_testroms
  export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS="$CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS"
  local cargo_cmd
  cargo_cmd="$(command -v cargo)"
  CARGO="$cargo_cmd" with_rustup_toolchain wasm-pack build --target web crates/06-apps/gbx-wasm \
    --out-dir ../../../web/pkg --out-name fabric_worker_wasm \
    -- -Z build-std=std,panic_abort
  wasm-tools print web/pkg/fabric_worker_wasm_bg.wasm | grep -E "(import.*memory)" | head -1
}

test_workspace() {
  assets_testroms
  export RUSTFLAGS="$NATIVE_RUSTFLAGS"
  cargo test --all-targets
  cargo test -p transport-fabric --features proptest --test mailbox_tests
}

test_golden() {
  assets_testroms
  export RUSTFLAGS="$NATIVE_RUSTFLAGS"
  cargo test -p tests transport_schema_goldens_v1
  cargo test -p tests inspector_ndjson_matches_golden
}

test_fast() {
  assets_testroms
  export RUSTFLAGS="$NATIVE_RUSTFLAGS"
  cargo nextest run --profile fast
}

test_slow() {
  assets_testroms
  export RUSTFLAGS="$NATIVE_RUSTFLAGS"
  cargo nextest run --profile slow --run-ignored ignored-only --features loom
}

test_wasm_smoke() {
  local wasm_port; wasm_port="$(calc_port)"
  note "wasm smoke on port $wasm_port"
  build_fabric_worker_wasm
  rm -rf tests/wasm/pkg
  mkdir -p tests/wasm/pkg
  cp web/pkg/fabric_worker_wasm.js tests/wasm/pkg/
  cp web/pkg/fabric_worker_wasm_bg.wasm tests/wasm/pkg/
  cp web/pkg/fabric_worker_wasm_bg.wasm.d.ts tests/wasm/pkg/ 2>/dev/null || true
  cp web/pkg/fabric_worker_wasm.d.ts tests/wasm/pkg/ 2>/dev/null || true
  cp web/worker.js tests/wasm/pkg/
  npm install --silent >/dev/null
  bash scripts/run-browser-test.sh tests/wasm "$wasm_port" tests/wasm_browser_test.js
}

test_wasm_light() {
  note "wasm node smoke test"
  build_fabric_worker_wasm
  node tests/wasm_node_smoke.js web/pkg/fabric_worker_wasm_bg.wasm
}

test_demo() {
  local base demo
  base="$(calc_port)"
  demo=$((base + 1))
  note "demo test on port $demo"
  build_fabric_worker_wasm
  npm install --silent >/dev/null
  bash scripts/run-browser-test.sh web "$demo" tests/demo_browser_test.js
}

ci_parity() {
  format_check
  lint_workspace
  build_workspace
  build_wasm_app
  test_fast
  test_wasm_light
}

usage() {
  cat <<'EOF'
Usage: scripts/tasks/run.sh <task>
Tasks:
  assets:testroms
  format:workspace
  format:check
  lint:workspace
  build:workspace
  build:wasm
  build:fabric-worker-wasm
  test:workspace
  test:golden
  test:fast
  test:slow
  test:wasm-smoke
  test:wasm-light
  test:demo
  ci-parity
EOF
}

case "${1:-help}" in
  "assets:testroms") assets_testroms ;;
  "format:workspace") format_workspace ;;
  "format:check")     format_check ;;
  "lint:workspace")   lint_workspace ;;
  "build:workspace")  build_workspace ;;
  "build:wasm")       build_wasm_app ;;
  "build:fabric-worker-wasm") build_fabric_worker_wasm ;;
  "test:workspace")   test_workspace ;;
  "test:golden")      test_golden ;;
  "test:fast")        test_fast ;;
  "test:slow")        test_slow ;;
  "test:wasm-smoke")  test_wasm_smoke ;;
  "test:wasm-light")  test_wasm_light ;;
  "test:demo")        test_demo ;;
  "ci-parity")        ci_parity ;;
  *) usage; exit 2 ;;
esac
