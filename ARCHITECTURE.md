# Layered Workspace Map

This repository models the GBX stack as numbered layers. Each directory tier
encodes the allowed dependency edges to keep compile-time boundaries explicit.

```
Allowed deps:
  01-transport → (none)
  02-runtime   → 01-transport
  03-driver    → 01-transport
  04-services  → 03-driver | 05-app-loop (types like gbx-frame) | 01-transport (reply payloads)
  05-app-loop  → 03-driver
  06-apps      → 05-app-loop | 03-driver | 02-runtime
  99-tests     → any (test-only)
```

Lower-numbered layers must never depend on higher-numbered layers. Tests remain
free to import any crate for coverage or scaffolding.
