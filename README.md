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
   devenv tasks run build:workspace      # native crates
   cargo build --target wasm32-unknown-unknown -p app  # WASM artifact
   ```

## Repository Layout

```
crates/
  hub/         # submit policies, message enums, services hub
  world/       # Elm-style reducers + World state
  app/         # scheduler, priority queues, intent/report loop
  services/    # non-blocking service stubs (kernel, gpu, audio, fs)
  transport/   # utility queues and shared primitives
  mock/        # mock services for tests
  tests/       # integration-style tests
docs/
  architecture.md  # authoritative design reference
  scaffold.md      # devenv tasks + engineering notes
web/               # WASM host, Trunk config (future UI runtime)
```

## Development Workflow

- **Formatting & linting**: enforced via `cargo fmt` and `cargo clippy -D warnings`. Run the `format:workspace` and `lint:workspace` tasks before pushing.
- **Testing**: `devenv tasks run test:workspace` executes unit and integration tests. Add new coverage under `crates/tests/src/` with descriptive names (e.g., `scheduler_lossless_requeue`).
- **SIMD/Web builds**: ensure `rustup target add wasm32-unknown-unknown` (already handled by devenv). Web pipelines will later use Trunk; see `docs/scaffold.md` for planned tasks.
- **Architecture alignment**: new commands, reports, or services must update both the code and the relevant sections in `docs/architecture.md`. Maintain the default submit policies via `default_policy()` implementations.

## Contributing

- Review `AGENTS.md` for contributor expectations (commit format, PR hygiene).
- Keep services non-blocking and express backpressure through `SubmitOutcome`.
- Pair code changes with tests that exercise the scheduler’s budgets (`DEFAULT_INTENT_BUDGET`, `DEFAULT_REPORT_BUDGET`) or the new behavior.
- For questions or proposals, open a discussion referencing the relevant architecture subsection (e.g., “§3 Backpressure Policies”).

## License

MIT © 2025 GBX contributors.
