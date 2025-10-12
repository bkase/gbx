# Repository Guidelines

## Project Structure & Module Organization
- `crates/hub`, `crates/world`, and `crates/app` hold the core Elm-style scheduler, reducers, and message types.
- `crates/services/*` provide non-blocking service shims (kernel, GPU, audio, FS); replace or extend these for real backends.
- Shared utilities live in `crates/transport`; test-only scaffolding is in `crates/mock`.
- Integration-style tests reside in `crates/tests`; architecture and scaffolding notes are under `docs/`.
- Web assets (WASM host stubs, Trunk config) belong in `web/`; native build artifacts land in `target/`.

## Build, Test, and Development Commands
- `devenv shell` — enter the reproducible Rust toolchain environment (direnv auto-loads it when available).
- `devenv tasks run format:workspace` — apply `rustfmt` across the workspace.
- `devenv tasks run lint:workspace` — run `cargo clippy --all-targets --all-features -D warnings`.
- `devenv tasks run build:workspace` — compile all workspace crates for native targets.
- `devenv tasks run test:workspace` — execute `cargo test` for every crate, including integration scenarios.

## Coding Style & Naming Conventions
- Rust edition 2021, four-space indentation, and `rustfmt` are mandatory; configure editors to format on save.
- Prefer `snake_case` for functions/variables, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants.
- Command/intent/report enums should mirror names in `docs/architecture.md`; add new variants with descriptive comments.
- Keep public APIs minimal; document tricky concurrency decisions inline with concise comments.

## Testing Guidelines
- Default tests use Rust’s built-in harness; add new cases under `crates/tests/src/` and mock services via `crates/mock`.
- Name tests after the behavior under scrutiny (`component_action_expectedOutcome`).
- When adding services or reducers, include regression coverage that exercises backpressure policies (e.g., WouldBlock requeues).
- Run `devenv tasks run test:workspace` before submitting any change; add targeted `cargo test -p <crate>` invocations when iterating.

## Commit & Pull Request Guidelines
- Use imperative present-tense commit subjects (`Add scheduler retry path`); group related changes per commit.
- Ensure every commit builds and passes tests; re-run `format:workspace` and `lint:workspace` prior to pushing.
- Pull requests should explain the user impact, reference relevant architecture sections, and link issues or design notes when available.
- Provide screenshots or CLI transcripts when altering developer workflows or observable behavior (e.g., new tasks or flags).

## Architecture & Services Primer
- Scheduler budgets (`DEFAULT_INTENT_BUDGET`, `DEFAULT_REPORT_BUDGET`) encode frame pacing—update them alongside tests.
- Service implementations must remain non-blocking; surface backpressure via `SubmitOutcome` and use deferred intents for retries.
