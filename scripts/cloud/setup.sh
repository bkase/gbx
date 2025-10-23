#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

RUST_TOOLCHAIN="$(cat scripts/env/RUST_TOOLCHAIN)"
NATIVE_RUSTFLAGS="$(cat scripts/env/NATIVE_RUSTFLAGS)"
WASM_RUSTFLAGS="$(cat scripts/env/WASM_RUSTFLAGS)"
export RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN"

BIN_DIR="$HOME/.local/bin"
mkdir -p "$BIN_DIR"
case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) ;;
  *) PATH="$PATH:$HOME/.cargo/bin" ;;
esac
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) PATH="$PATH:$BIN_DIR" ;;
esac
export PATH
export DEBIAN_FRONTEND=noninteractive
TMP_ROOT="$HOME/.tmp"
mkdir -p "$TMP_ROOT"

note() { printf "\033[1;34m[setup]\033[0m %s\n" "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
maybe_sudo() {
  if command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    "$@"
  fi
}

persist_env() {
  if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
    {
      echo "RUSTUP_TOOLCHAIN=\"$RUST_TOOLCHAIN\""
      echo "NATIVE_RUSTFLAGS=\"$NATIVE_RUSTFLAGS\""
      echo "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS=\"$WASM_RUSTFLAGS\""
      echo 'PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"'
    } >> "$CLAUDE_ENV_FILE"
  fi

  local rc="$HOME/.bashrc"
  touch "$rc"
  if ! grep -q 'NATIVE_RUSTFLAGS=' "$rc"; then
    {
      echo "export RUSTUP_TOOLCHAIN=\"$RUST_TOOLCHAIN\""
      echo "export NATIVE_RUSTFLAGS=\"$NATIVE_RUSTFLAGS\""
      echo "export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS=\"$WASM_RUSTFLAGS\""
      echo 'export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"'
    } >> "$rc"
  fi
}

apt_install_basics() {
  if have apt-get; then
    maybe_sudo apt-get update -y
    maybe_sudo apt-get install -y curl unzip ca-certificates python3 git gnupg xz-utils
    maybe_sudo apt-get install -y chromium || true
    if ! have chromium && ! have chromium-browser && ! have google-chrome; then
      note "Installing Google Chrome (fallback)"
      curl -fsSL https://dl.google.com/linux/linux_signing_key.pub | maybe_sudo gpg --dearmor -o /usr/share/keyrings/google-linux.gpg
      echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-linux.gpg] https://dl.google.com/linux/chrome/deb/ stable main" \
        | maybe_sudo tee /etc/apt/sources.list.d/google-chrome.list >/dev/null
      maybe_sudo apt-get update -y
      maybe_sudo apt-get install -y google-chrome-stable || true
    fi
    maybe_sudo apt-get clean
    maybe_sudo rm -rf /var/lib/apt/lists/*
  fi
}

install_node_corepack() {
  if ! have node; then
    export NVM_DIR="$HOME/.nvm"
    if [ ! -s "$NVM_DIR/nvm.sh" ]; then
      curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
    fi
    # shellcheck disable=SC1090
    . "$NVM_DIR/nvm.sh"
    nvm install --lts
  fi
  corepack enable || true
}

install_rust() {
  if ! have rustup; then
    curl -fsSL https://sh.rustup.rs | sh -s -- -y
  fi
  export PATH="$HOME/.cargo/bin:$PATH"
  rustup toolchain install "$RUST_TOOLCHAIN" -c rustc -c cargo -c rustfmt -c clippy -c rust-src -c rust-analyzer
  rustup override set "$RUST_TOOLCHAIN"
  rustup target add wasm32-unknown-unknown --toolchain "$RUST_TOOLCHAIN"
}

install_wasm_pack_binary() {
  if have wasm-pack; then
    return
  fi
  local version="0.13.1"
  local os="$(uname -s)"
  local arch="$(uname -m)"
  local tarball=""
  local folder=""
  if [ "$os" = "Linux" ] && [ "$arch" = "x86_64" ]; then
    tarball="wasm-pack-v${version}-x86_64-linux.tar.gz"
    folder="wasm-pack-v${version}-x86_64-linux"
  elif [ "$os" = "Linux" ] && [ "$arch" = "aarch64" ]; then
    tarball="wasm-pack-v${version}-aarch64-linux.tar.gz"
    folder="wasm-pack-v${version}-aarch64-linux"
  else
    return 1
  fi
  local url="https://github.com/rustwasm/wasm-pack/releases/download/v${version}/${tarball}"
  local tmpdir; tmpdir="$(mktemp -d)"
  curl -fsSL "$url" -o "$tmpdir/wasm-pack.tar.gz"
  tar -xzf "$tmpdir/wasm-pack.tar.gz" -C "$tmpdir"
  install -m 0755 "$tmpdir/$folder/wasm-pack" "$BIN_DIR/wasm-pack"
  rm -rf "$tmpdir"
  note "Installed wasm-pack v${version} binary."
}

install_nextest_binary() {
  if have cargo-nextest; then
    return
  fi
  local version="0.9.108"
  local os="$(uname -s)"
  local arch="$(uname -m)"
  local tarball=""
  local folder=""
  if [ "$os" = "Linux" ] && [ "$arch" = "x86_64" ]; then
    tarball="cargo-nextest-v${version}-x86_64-unknown-linux-gnu.tar.gz"
    folder="cargo-nextest-v${version}-x86_64-unknown-linux-gnu"
  elif [ "$os" = "Linux" ] && [ "$arch" = "aarch64" ]; then
    tarball="cargo-nextest-v${version}-aarch64-unknown-linux-gnu.tar.gz"
    folder="cargo-nextest-v${version}-aarch64-unknown-linux-gnu"
  else
    return 1
  fi
  local url="https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-v${version}/${tarball}"
  local tmpdir; tmpdir="$(mktemp -d)"
  curl -fsSL "$url" -o "$tmpdir/nextest.tar.gz"
  tar -xzf "$tmpdir/nextest.tar.gz" -C "$tmpdir"
  if [ -f "$tmpdir/$folder/cargo-nextest" ]; then
    install -m 0755 "$tmpdir/$folder/cargo-nextest" "$BIN_DIR/cargo-nextest"
    note "Installed cargo-nextest v${version} binary."
  else
    return 1
  fi
  rm -rf "$tmpdir"
}

install_wasm_tools_binary() {
  if have wasm-tools; then
    return
  fi
  local version="1.221.1"
  local os="$(uname -s)"
  local arch="$(uname -m)"
  local tarball=""
  if [ "$os" = "Linux" ] && [ "$arch" = "x86_64" ]; then
    tarball="wasm-tools-${version}-x86_64-linux.tar.xz"
  elif [ "$os" = "Linux" ] && [ "$arch" = "aarch64" ]; then
    tarball="wasm-tools-${version}-aarch64-linux.tar.xz"
  else
    return 1
  fi
  local url="https://github.com/bytecodealliance/wasm-tools/releases/download/v${version}/${tarball}"
  local tmpdir; tmpdir="$(mktemp -d)"
  curl -fsSL "$url" -o "$tmpdir/wasm-tools.tar.xz"
  tar -xf "$tmpdir/wasm-tools.tar.xz" -C "$tmpdir"
  find "$tmpdir" -name wasm-tools -type f -exec install -m 0755 {} "$BIN_DIR/wasm-tools" \;
  rm -rf "$tmpdir"
  note "Installed wasm-tools v${version} binary."
}

ensure_cli_tools() {
  install_wasm_pack_binary || {
    local tmp_dir
    tmp_dir="$(mktemp -d "${TMP_ROOT}/wasm-pack.XXXXXX")"
    CARGO_TARGET_DIR="$tmp_dir" cargo install --locked wasm-pack || true
    rm -rf "$tmp_dir"
  }
  install_nextest_binary || {
    local tmp_dir
    tmp_dir="$(mktemp -d "${TMP_ROOT}/nextest.XXXXXX")"
    CARGO_TARGET_DIR="$tmp_dir" cargo install --locked cargo-nextest || true
    rm -rf "$tmp_dir"
  }
  install_wasm_tools_binary || {
    local tmp_dir
    tmp_dir="$(mktemp -d "${TMP_ROOT}/wasm-tools.XXXXXX")"
    CARGO_TARGET_DIR="$tmp_dir" cargo install --locked wasm-tools || true
    rm -rf "$tmp_dir"
  }
  rm -rf "$HOME/.cargo/registry/index" "$HOME/.cargo/registry/cache" "$HOME/.cargo/git"
  for cmd in wasm-pack cargo-nextest wasm-tools; do
    if have "$cmd"; then
      local path
      path="$(command -v "$cmd")"
      note "Using $path"
    else
      note "Warning: $cmd not installed"
    fi
  done
  for cmd in wasm-pack cargo-nextest wasm-tools; do
    if ! have "$cmd"; then
      note "Error: required tool $cmd is unavailable after setup"
      exit 1
    fi
  done
}

npm_bootstrap() {
  if [ -f package-lock.json ]; then
    npm ci --no-audit --fund=false
  else
    npm install --no-audit --fund=false
  fi
}

check_browser() {
  if have google-chrome; then
    note "Chrome: $(google-chrome --version)"
  elif have chromium; then
    note "Chromium: $(chromium --version)"
    export CHROME_BIN="$(command -v chromium)"
  elif have chromium-browser; then
    note "Chromium: $(chromium-browser --version)"
    export CHROME_BIN="$(command -v chromium-browser)"
  else
    note "No Chrome/Chromium found; wasm browser tests may fail."
  fi
}

main() {
  note "Bootstrapping cloud env (Claude/Codex)"
  apt_install_basics
  install_node_corepack
  install_rust
  ensure_cli_tools
  npm_bootstrap
  check_browser
  persist_env

  note "rustc: $(rustc --version)"
  note "cargo: $(cargo --version)"
  note "node:  $(node --version)"
  note "npm:   $(npm --version)"
}

main "$@"
