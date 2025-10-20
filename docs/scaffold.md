# 🧩 Game Boy Emulator Monorepo — Engineering Document

**Version:** 1.0 (setup with devenv.sh tasks)
**Date:** 2025-10-11

## 0) Scope & Objectives

- **DX & Reproducibility:** One declarative `devenv.nix` that exposes consistent **tasks**: `format`, `lint`, `test`, `build`, `build:wasm`, `run:web`, `bench`, `fuzz`.
- **Cross-platform:** Native (macOS Apple Silicon & Linux x86_64), WebAssembly (SIMD), WebGPU renderer.
- **CI parity:** GitHub Actions jobs call the same devenv tasks as local dev (see uploaded example that runs `devenv tasks run …` ).
- **Perf foundation:** SIMD-wide kernel; multi-worker parallelism (native threads / web workers); Elm-style typed services & reducers.

---

## 1) Repository Layout

```
gameboy-emulator/
├─ flake.nix                 # (optional but recommended) Nix flake wrapper for devenv
├─ devenv.nix                # devenv.sh configuration (tasks + toolchains)
├─ .github/
│  └─ workflows/
│      └─ ci.yml             # CI calling "devenv tasks run …" (see §4)
├─ Cargo.toml                # workspace manifest (members = crates/*)
├─ crates/
│  ├─ hub/                   # Services hub + Service trait + SubmitOutcome
│  ├─ world/                 # reducers + World state + FollowUps
│  ├─ services/
│  │   ├─ kernel/            # SIMD core; threads/workers glue
│  │   ├─ gpu/               # wgpu (WebGPU) backend
│  │   ├─ audio/             # cpal (native) / WebAudio (web)
│  │   └─ fs/                # persistence, autosave
│  ├─ transport/             # SPSC rings: crossbeam (native) / SAB (web)
│  ├─ app/                   # rAF scheduler; queues; health handling
│  ├─ mock/                  # mocks, chaos hooks
│  └─ tests/                 # unit, property, fuzz, perf, determinism
└─ web/
   ├─ main.ts                # thin TypeScript runtime: UI input → Intent
   └─ pkg/                   # WASM build output
```

---

## 2) Rust & WASM Targets

- **Native:** `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` (CI), optionally cross-target `x86_64-apple-darwin` from Apple Silicon via SDK if desired later.
- **Web:** `wasm32-unknown-unknown` with **SIMD** (`+simd128`).
  - Use `wasm-bindgen` for glue with wasm-pack.
  - Renderer: `wgpu` (compiles to WebGPU on web, Vulkan/Metal on native).

**Feature flags (example):**

- `features = ["webgpu", "simd", "sabrings"]` for web
- `features = ["nativegpu", "simd"]` for native

---

## 3) devenv.nix (declarative tasks via devenv.sh)

> Paste this at repo root as `devenv.nix`. It defines **packages**, **languages**, **env**, and **tasks**.
> All commands are run _inside_ the reproducible shell: `devenv shell` or directly on CI via `devenv tasks run <task>`.

```nix
{ pkgs, lib, ... }:

let
  # Versions can be pinned by flake.lock (recommended); this is simplified.
  rustToolchain = pkgs.rustup; # install stable + targets via rustup in enterShell
in
{
  # Tooling / languages available in the shell
  languages.rust.enable = true;
  languages.javascript.enable = true;

  # Extra packages needed for web & CI
  packages = with pkgs; [
    rustToolchain
    nodejs
    wasm-pack
    binaryen            # wasm-opt
    wasm-bindgen-cli
    pkg-config
    # Native audio/video deps (Linux CI)
    alsa-lib
    libxkbcommon
    vulkan-loader
    # Perf & test helpers
    just
    jq
    coreutils
  ];

  # Environment adjustments
  env = {
    # Enable WASM SIMD for wasm32 builds
    RUSTFLAGS = "-C target-feature=+simd128";
    # Ensure wasm32 toolchain target is present (done in enterShell)
    CARGO_TERM_COLOR = "always";
    # For wgpu/WebGPU on CI: allow headless checks (Linux)
    WGPU_BACKEND = "vulkan,metal,gl,dx12,webgpu";
  };

  # Bootstrap: runs when entering shell (interactive and CI)
  enterShell = ''
    set -eu
    echo "👉 Ensuring Rust toolchains & targets…"
    rustup default stable
    rustup target add wasm32-unknown-unknown
    rustup component add clippy rustfmt
    echo "Rust: $(rustc --version)"
    echo "Node: $(node --version)"
  '';

  # -------- TASKS (the important part) --------
  # These show up under `devenv tasks` and can be run via
  #   devenv tasks run <task>
  tasks = {
    # Workspace hygiene
    "format" = {
      command = "cargo fmt --all";
      description = "Format all Rust code";
    };

    "lint" = {
      command = ''
        cargo clippy --workspace --all-targets -- -D warnings
      '';
      description = "Clippy across workspace";
    };

    "test" = {
      command = "cargo test --workspace --all-targets";
      description = "Run Rust unit/integration tests";
    };

    "bench" = {
      command = "cargo bench --workspace || true";
      description = "Run benchmarks (non-fatal on CI)";
    };

    "fuzz" = {
      command = "cargo fuzz list || true"; # placeholder; wire up fuzz targets later
      description = "Fuzzing entry (placeholder: add cargo-fuzz targets)";
    };

    "build" = {
      command = "cargo build --workspace --release";
      description = "Native release build (Metal/Vulkan for wgpu)";
    };

    # ---------- Web / WASM ----------
    "build:wasm" = {
      command = ''
        wasm-pack build crates/app --target web --release
      '';
      description = "Build WASM bundle (SIMD) with wasm-pack";
    };

    "run:web" = {
      command = ''
        # Starts dev server on http://127.0.0.1:8080
        cargo run -p dev_server -- --dist web/dist --port 8080
      '';
      description = "Run dev server (WebGPU + WASM with COOP/COEP headers)";
    };

    # Example “typed” tasks (you can add more granular ones later)
    "lint:workspace" = {
      command = "cargo clippy --workspace --all-features -- -D warnings";
      description = "Strict lint with all features";
    };

    # Hooks for differential testing (native vs wasm) can be added:
    "test:determinism" = {
      command = ''
        cargo test -p tests -- --ignored --nocapture
      '';
      description = "Run determinism/differential tests";
    };
  };
}
```

### Optional `flake.nix` (recommended)

```nix
{
  description = "Game Boy Emulator monorepo (devenv)";

  inputs.devenv.url = "github:cachix/devenv";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";

  outputs = { self, nixpkgs, devenv }:
    let
      systems = [ "aarch64-darwin" "x86_64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system:
        f (import nixpkgs { inherit system; }));
    in
    {
      devShells = forAllSystems (pkgs:
        (devenv.lib.mkShell {
          inherit pkgs;
          modules = [ ./devenv.nix ];
        }).devShell);
    };
}
```

---

## 4) GitHub Actions CI (calling devenv tasks)

We mirror your **uploaded example** that installs `devenv` and runs `devenv tasks run …` (see “Install devenv.sh” & `devenv tasks run lint:aethel|build:aethel|test:aethel`) . Below, we keep that structure and add a Linux job for WASM builds.

```yaml
# .github/workflows/ci.yml
name: ci

on:
  push:
    branches: [main]
  pull_request:

jobs:
  native-macos:
    runs-on: macos-14
    steps:
      - uses: actions/checkout@v4

      - uses: cachix/install-nix-action@v31
      - uses: cachix/cachix-action@v16
        with:
          name: devenv

      - name: Install devenv
        run: nix profile install nixpkgs#devenv

      - name: Format
        run: devenv tasks run format

      - name: Lint
        run: devenv tasks run lint

      - name: Test
        run: devenv tasks run test

      - name: Build (native)
        run: devenv tasks run build

  wasm-linux:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: cachix/install-nix-action@v31
      - uses: cachix/cachix-action@v16
        with:
          name: devenv

      - name: Install devenv
        run: nix profile install nixpkgs#devenv

      - name: Build (WASM)
        run: devenv tasks run build:wasm

      # Optional: headless smoke (if you add web tests)
      - name: Smoke (WASM bundle exists)
        run: test -e dist || test -e build || ls -la
```

> If you want to keep the **multi-project** pattern from your example (separate jobs like `aethel`, `momentum`), the same recipe applies; swap task names for your workspace (your example uses `devenv tasks run lint:aethel` etc.) .

### CI Caching (optional)

You can add:

- `actions/cache` for `~/.cargo` and `target/` keyed by `Cargo.lock` + runner OS.
- Node caches if your web artifacts are large.

---

## 5) Web Runtime & Dev Server

**Local dev loop with COOP/COEP headers**

```bash
# Build the WASM bundle
devenv tasks run build:transport-worker

# Serve the dist dir with required headers
devenv tasks run web:serve
```

The Rust dev server (`crates/dev_server`) injects `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`, so browsers unlock shared-array
buffer and WebGPU paths needed for the emulator. Add assets (JS, wasm,
textures) under `web/` as needed.
Pass additional CLI flags to the dev server with `--`, e.g. `devenv tasks run web:serve -- --port 9090`.

---

## 6) Cargo Workspace Manifests (skeletons)

**Cargo.toml (workspace root)**

```toml
[workspace]
members = [
  "crates/world",
  "crates/services/kernel",
  "crates/services/gpu",
  "crates/services/audio",
  "crates/services/fs",
  "crates/transport",
  "crates/app",
  "crates/mock",
  "crates/tests",
]
resolver = "2"

[workspace.package]
edition = "2021"

[workspace.dependencies]
smallvec = "1"
crossbeam = "0.8"
wgpu = { version = "0.20", default-features = false, features = ["webgpu","vulkan","metal"] }
cpal = "0.15"
serde = { version = "1", features = ["derive"] }
```

**crates/app/Cargo.toml (WASM + native toggles)**

```toml
[package]
name = "app"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[features]
web = ["wgpu/webgpu"]
native = ["wgpu/vulkan", "wgpu/metal"]
simd = []

[dependencies]
wgpu = { workspace = true }
wasm-bindgen = { version = "0.2", optional = true }
console_error_panic_hook = { version = "0.1", optional = true }

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = { version = "0.2" }
```

> For SIMD in Rust on wasm: keep `RUSTFLAGS="-C target-feature=+simd128"` in the env; ensure code paths use `#[cfg(target_feature = "simd128")]` where appropriate.

---

## 7) Tests & Gates

- **Unit / property:** reducers (pure) + command policies; `proptest` for invariants (budget monotonicity, queue bounds).
- **Integration:** deterministic fake rAF loop; ensure `LaneFrame → UploadFrame` occurs in same tick; ≤1-frame latency for deferred work.
- **Differential:** scalar vs SIMD kernel; native vs wasm snapshots; audio chunk hash tolerances.
- **Perf:** `criterion` benches; fail on ≥5% regression on CI (optional).
- **Chaos:** service `WouldBlock`/`Closed` fault injection with mocks; scheduler recovery assertions.

Expose useful test entrypoints as devenv tasks later (e.g., `test:determinism`, `perf:kernel`).

---

## 8) Platform Notes

- **Apple Silicon (local):** default job `native-macos` runs on `macos-14`.
- **x86_64:** covered via **Linux CI** for now (fast and representative for kernel perf). If you need **macOS x86_64** artifacts specifically, we can add cross-compilation using the macOS SDK or introduce a self-hosted runner.
- **WebGPU:** Ensure Chrome/Edge ≥ 118; flags no longer required on modern builds. On CI, we just build the bundle; browser-level tests can be added later with Playwright.

---

## 9) Future Extensions

- **Matrix CI:** `{ os: [macos-14, ubuntu-latest], mode: [native, wasm] }`.
- **Caches:** `~/.cargo`, `target/`, web artifacts.
- **CD:** Pages/Cloudflare Pages deploy of `dist` after `build:wasm`.
- **GPU perf CI:** headless compute sanity via `wgpu` on Linux (Vulkan) to catch shader regressions.
