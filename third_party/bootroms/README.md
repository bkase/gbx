# DMG Boot ROM

This repository does not ship Nintendo boot ROM images. To enable the DMG
bootstrap sequence in the emulator, drop your legally obtained `dmg.bin`
into this directory or point `GBX_BOOT_ROM_DMG` at the file:

```sh
export GBX_BOOT_ROM_DMG=/absolute/path/to/dmg.bin
```

The expected file length is 256 bytes. If the file is missing the emulator
falls back to the post-boot state so tests still pass.
