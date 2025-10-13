# Contributing to GBX

## Test Discipline

We follow a strict two-bucket testing approach to keep the test suite fast and reliable.

### Test Categories

#### Fast Tests (Default)

- **Default behavior**: All tests without `#[ignore]` are fast tests
- **Run on**: Every PR via CI
- **Target timing**: ≤100 ms per test locally
- **Hard cap**: 20 seconds per test (enforced by Nextest)
- **Reclassification rule**: If a test consistently exceeds ~200 ms in CI, move it to slow

**Guidelines for fast tests**:
- Avoid sleeps or time-based waits
- Keep allocations small; prefer stack/SmallVec
- Use bounded property tests (32 cases by default via `PROPTEST_CASES`)
- Focus on unit-level behavior

**Example**:
```rust
#[test]
fn parses_small_message_fast() {
    let msg = parse_message(&[0x01, 0x02]);
    assert!(msg.is_ok());
}
```

#### Slow Tests (Opt-in)

- **Marking**: Add `#[ignore]` attribute and prefix name with `slow_`
- **Run on**: Nightly builds, manual runs, or local on-demand
- **Hard cap**: Still 20 seconds per test
- **Purpose**: Deep exploration, stress tests, heavy property testing

**Example**:
```rust
#[test]
#[ignore]
fn slow_stress_many_frames() {
    // Stress test with 1000 iterations
    for _ in 0..1000 {
        // ... heavy workload
    }
}
```

### Property Testing

#### Fast Lane (Bounded)
```rust
use proptest::prelude::*;

const FAST_CASES: u32 = 32;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: FAST_CASES,
        timeout: 2_000, // ms (still under 20s global cap)
        .. ProptestConfig::default()
    })]

    #[test]
    fn property_test_fast(input in any::<Vec<u8>>()) {
        // Fast property test
    }
}
```

#### Slow Lane (Deep)
```rust
#[test]
#[ignore]
fn slow_property_test_deep() {
    use proptest::prelude::*;
    let cfg = ProptestConfig {
        cases: 2000,
        timeout: 20_000,
        ..Default::default()
    };
    proptest::test_runner::TestRunner::new(cfg)
        .run(&any::<Vec<u8>>(), |input| {
            // Deep property exploration
            Ok(())
        })
        .unwrap();
}
```

### Running Tests

**Local development** (fast loop):
```bash
devenv tasks run test:fast
```

**Local deep checks**:
```bash
devenv tasks run test:slow
```

**Legacy command** (still works):
```bash
devenv tasks run test:workspace
```

### CI Integration

- **PR builds**: Run `test:fast` only (fast feedback)
- **Nightly builds**: Run `test:slow` (deep validation)
- Both enforce the 20-second hard cap per test

### Hard Limits

- **Per-test timeout**: 20 seconds (hard cap, enforced by `nextest.toml`)
- **Fast test target**: ≤100 ms locally
- **Fast test threshold**: Consistently >200 ms in CI → reclassify as slow

### Configuration Files

- `.config/nextest.toml` - Nextest profiles (fast/slow)
- `devenv.nix` - Task definitions and environment variables
- Environment variables:
  - `PROPTEST_CASES=32` - Default cases for fast property tests
  - `PROPTEST_TIMEOUT=2000` - Default timeout (ms) for property tests

## Development Setup

This project uses [devenv](https://devenv.sh/) for reproducible development environments.

```bash
# Enter the development shell
devenv shell

# Run format check
devenv tasks run format:check

# Run linter
devenv tasks run lint:workspace

# Run fast tests
devenv tasks run test:fast

# Build workspace
devenv tasks run build:workspace
```

## Git Hooks

Pre-commit hooks are automatically installed and will run:
- Format check
- Linter (clippy)
- Fast test suite

Pre-push hooks will run:
- Full workspace build (native + wasm)

## Pull Request Guidelines

1. Ensure all fast tests pass locally
2. Add tests for new functionality
3. Keep individual tests fast (≤100 ms target)
4. Mark heavy tests with `#[ignore]` and prefix `slow_`
5. Run `devenv tasks run test:fast` before pushing
6. CI will validate format, lint, build, and fast tests
