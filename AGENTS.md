# Repository Guidelines

## Project Structure & Module Organization
- `crates/hub` provides the services hub, shared scheduling traits, and default budgets; `crates/world` exports the message schema plus reducers; `crates/app` hosts the Elm-style scheduler built on top.
- `crates/services/*` provide non-blocking service shims (kernel, GPU, audio, FS); replace or extend these for real backends.
- Shared utilities live in `crates/transport`; test-only scaffolding is in `crates/mock`.
- Integration-style tests reside in `crates/tests`; architecture and scaffolding notes are under `docs/`.
- Web assets (WASM host stubs) belong in `web/`; native build artifacts land in `target/`.

## Build, Test, and Development Commands
- `devenv shell` — enter the reproducible Rust toolchain environment (direnv auto-loads it when available).
- `devenv tasks run format:workspace` — apply `rustfmt` across the workspace.
- `devenv tasks run lint:workspace` — run `cargo clippy --all-targets --all-features -D warnings -D clippy::undocumented_unsafe_blocks` (every `unsafe` block must carry a `// SAFETY:` comment explaining the invariants).
- `devenv tasks run build:workspace` — compile all workspace crates for native targets.
- `devenv tasks run test:workspace` — execute `cargo test` for every crate, including integration scenarios.
- Add a `devenv` task for every new developer workflow before documenting raw commands elsewhere.
- **Always enter `devenv shell` for wasm builds/tests.** The shared-memory/atomics linker flags now live only in `devenv.nix`; invoking `cargo build --target wasm32-unknown-unknown` outside the shell will produce an incompatible artifact.

## Collaboration Workflow
- Expect concurrent teammates in this workspace; keep your edits scoped so parallel sessions do not conflict.
- Stage only the files you personally touched during your session.
- Run targeted tests that cover the files you created or modified before handing off.
- When committing, include only the staged files you worked on in that session.

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
- Transport ABI goldens live under `crates/tests/golden/*.bin` and are verified by `devenv tasks run test:golden` (also run via the commit hook). Regenerate fixtures after intentional schema changes with `UPDATE_GOLDEN=1 devenv tasks run test:golden`, then commit the refreshed `.bin` files together with the schema bump.

### Lockstep Oracle Workflow
- Harness lives at `crates/04-services/kernel-core/tests/lockstep_sameboy.rs` and compares our scalar core against SameBoy via the `safeboy` bindings.
- Always iterate inside the reproducible toolchain: `devenv shell cargo test -p kernel-core lockstep_cpu_instrs_02_interrupts -- --nocapture` (swap the test name for other ROMs such as `03_op_sp_hl` once earlier divergences are fixed).
- On every test run the oracle prints the first divergence: register file snapshot, IE/IF flags, upcoming opcodes, and the last ~64 instruction boundaries for each core. Treat the first mismatch as **the** bug to chase; later differences cascade from it.
- Typical loop:
  1. Run the targeted lockstep test and capture the divergence (PC/opcode + history usually points at the precise instruction or interrupt edge).
  2. Patch the kernel core to fix the identified behavior (flags, timing, IF/IE masking, etc.).
  3. Re-run the same lockstep test until it passes silently (no output).
  4. Advance to the next ROM (`03`, `04`, …) and repeat; the harness makes it straightforward to burn down issues one at a time.
- Optional: add an integration test or regression case once a divergence is resolved so future edits don’t regress the same instruction path.

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
