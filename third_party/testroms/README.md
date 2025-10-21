# Test ROM Vendors

The actual Game Boy test ROM bundles are fetched on-demand. Run:

```sh
devenv tasks run assets:testroms
```

This downloads the pinned c-sp/game-boy-test-roms release into `third_party/testroms/c-sp-v7.0/`.
