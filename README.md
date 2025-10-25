# Game Boy Emulator (GBX)

Ultra-high-performance, Elm-inspired Game Boy emulator built around SIMD kernels, typed services, and a deterministic scheduler. This repository holds the native and WebAssembly codebase, reproducible development environment, and architecture documentation.

## Quick Start

1. **Prerequisites**  
   - Install [direnv](https://direnv.net/) and allow it (`direnv allow` in the repo).  
   - Install [devenv](https://devenv.sh/) v1.10+ (the direnv hook will prompt if missing).

2. **Enter the environment**  
   ```bash
   devenv shell
   ```
   or simply `cd` into the repository with direnv enabled.

3. **Run the hygiene pipeline**  
   ```bash
   devenv tasks run format:workspace
   devenv tasks run lint:workspace
   devenv tasks run test:workspace
   ```

4. **Build targets**  
 ```bash
 devenv tasks run build:workspace   # native crates
 devenv tasks run build:wasm        # WASM artifact
 ```

## ROM Assets

- **DMG boot ROM**: Place your legally obtained `dmg.bin` under `third_party/bootroms/` or set `GBX_BOOT_ROM_DMG=/absolute/path/to/dmg.bin`. If absent, the emulator skips the boot ROM and still passes the test suite.
- **Default cartridge**: By default the kernel boots against Blargg’s `cpu_instrs/individual/03-op sp,hl.gb` ROM from the vendored test bundle. Override it with `GBX_DEFAULT_ROM=/path/to/cart.gb` if you want a different startup cartridge. The workspace does not ship commercial ROMs like `tetris.gb`; fetch or supply your own and optionally point tests at it with `GBX_TETRIS_ROM=/path/to/tetris.gb`.

## Web Dev Server

- `devenv tasks run web:serve` — runs the Rust dev server that injects COOP/COEP headers so browsers enable SharedArrayBuffer/WebGPU.
- Adjust the port or host by passing extra args (for example `devenv tasks run web:serve -- --host 0.0.0.0 --port 9000`); the default bind is `127.0.0.1:8080`.

## Repository Layout

```
crates/
  hub/              # Services hub, submit policies, shared scheduling traits
  world/            # Message enums, reducers, and world state
  app/              # scheduler, priority queues, intent/report loop
  services/         # non-blocking service stubs (kernel, gpu, audio, fs)
  transport/        # utility queues and shared primitives
  transport-worker/ # Reusable WASM worker functions (app-agnostic)
  gbx-wasm/         # Top-level WASM module (re-exports worker + adds tests)
  mock/             # mock services for tests
  tests/            # native integration tests
docs/
  architecture.md  # authoritative design reference
  scaffold.md      # devenv tasks + engineering notes
web/               # WASM host artifacts (future UI runtime)
```

### WASM Architecture: Single Module Pattern

**Critical**: We use a **single WASM module** (`gbx-wasm`) with multiple `wasm_bindgen` entry points for both the main thread and workers. This avoids the `__wbindgen_start` deadlock that occurs with multiple separate WASM modules.

**Why**: wasm-bindgen's `__wbindgen_start` function performs threading initialization including atomic wait operations. When a worker tries to initialize a separate WASM module for the first time, these atomic waits deadlock because they expect a coordinator thread that doesn't exist in the worker-only context.

**Pattern** (inspired by [gbx-playground](https://github.com/bkase/gbx-playground)):
- Main thread calls `await init()` to initialize the module (runs `__wbindgen_start` in main context)
- Workers import the same module and call `await init(undefined, sharedMemory)` to re-initialize with shared memory
- **`transport-worker`** crate: Reusable, app-agnostic worker functions (no dependencies on app/hub/world)
- **`gbx-wasm`** crate: Top-level WASM module that re-exports `transport-worker` functions and adds GBX-specific test orchestration

**DO NOT** create separate WASM artifacts for workers and tests - this will cause initialization hangs. Keep all WASM entry points in the single `gbx-wasm` module.

**Generalization**: To use `transport-worker` in your own project:
1. Create your own top-level WASM crate (like `gbx-wasm`)
2. Add `transport-worker` as a dependency
3. Re-export its worker functions: `pub use fabric_worker_wasm::{worker_init, worker_flood, ...};`
4. Add your own `#[wasm_bindgen]` entry points for your app-specific functionality

## Development Workflow

- **Formatting & linting**: enforced via `cargo fmt` and `cargo clippy -D warnings`. Run the `format:workspace` and `lint:workspace` tasks before pushing.
- **Testing**: `devenv tasks run test:workspace` executes unit and integration tests. Add new coverage under `crates/tests/src/` with descriptive names (e.g., `scheduler_lossless_requeue`).
- **SIMD/Web builds**: ensure `rustup target add wasm32-unknown-unknown` (already handled by devenv). See `docs/scaffold.md` for planned tasks.
- **Architecture alignment**: new commands, reports, or services must update both the code and the relevant sections in `docs/architecture.md`. Maintain the default submit policies via `default_policy()` implementations.

## Contributing

- Review `AGENTS.md` for contributor expectations (commit format, PR hygiene).
- Keep services non-blocking and express backpressure through `SubmitOutcome`.
- Pair code changes with tests that exercise the scheduler’s budgets (`DEFAULT_INTENT_BUDGET`, `DEFAULT_REPORT_BUDGET`) or the new behavior.
- For questions or proposals, open a discussion referencing the relevant architecture subsection (e.g., “§3 Backpressure Policies”).

## License

MIT © 2025 GBX contributors.
