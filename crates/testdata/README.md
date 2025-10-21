# testdata crate

Dev-only crate that exposes vendored Game Boy test ROM metadata and byte loaders for the gbx workspace.

## Usage

Add as a dev-dependency from crates that need ROM access:

```toml
[dev-dependencies]
testdata = { path = "../testdata" }
```

From tests, request bytes by normalized path:

```rust
let rom_bytes = testdata::bytes("mooneye-test-suite/acceptance/ei_sequence.gb");
```

Call `testdata::list()` to enumerate metadata, or `testdata::metadata(path)` for a specific entry.

## Updating ROM bundles

1. Fetch the pinned bundle with `devenv tasks run assets:testroms`.
2. When upgrading, adjust the task to point at the new release/tag, then rerun it to download fresh contents.
3. Regenerate `crates/testdata/roms.index.toml` using `python3 scripts/generate_testrom_manifest.py`.
4. Re-run a build to let `build.rs` recreate its generated index.

The build script copies ROMs into `OUT_DIR` for native runs and optionally embeds the small suites when the crate is compiled with the `embed` feature (useful for wasm targets).
