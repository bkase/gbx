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
- Add a `devenv` task for every new developer workflow before documenting raw commands elsewhere.

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

## GitHub Issues & Project Board
- Project board lives at `https://github.com/users/bkase/projects/1` and is titled **gbx Project Kanban**; it tracks all work for this repo (including Milestone M1).
- Use `gh issue create --repo bkase/gbx --title "<title>" --label "<owner label>" --label M1 --body-file -` and pipe the markdown checklist body via stdin to create new track/sub-track issues without prompts.
- After creating or updating an issue, add it to the board with `gh project item-add <project_number> --owner @me --url https://github.com/bkase/gbx/issues/<num>` (project number is `1` right now).
- Set the kanban status with `gh project item-edit --id <item_id> --project-id PVT_kwHOAAfddc4BFY_I --field-id PVTSSF_lAHOAAfddc4BFY_Izg2vZBo --single-select-option-id f75ad846` for **Todo**, or swap the option id for other states (`47fc9ee4` = In Progress, `98236657` = Done).
- Query existing items via `gh project item-list 1 --owner @me --format json` to discover item ids before editing fields.
