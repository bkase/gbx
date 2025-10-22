# Layered Workspace Map

This repository models the GBX stack as numbered layers. Each directory tier
encodes the allowed dependency edges to keep compile-time boundaries explicit.

## Strict Rule

Higher-numbered layers may depend on any lower-numbered layer (and same-layer).
Lower-numbered layers must never depend on higher-numbered layers.

```
Layer rules:
  01-transport → 01 only (no higher layers)
  02-runtime   → 01-02 only
  03-driver    → 01-03 only
  04-services  → 01-04 only
  05-app-loop  → 01-05 only
  06-apps      → 01-06 (any lower layer)
  99-tests     → any (test-only)
```

This layering is enforced by `scripts/check-layer-deps.py` which runs on pre-commit.
