#!/usr/bin/env python3
"""
Validate that crate dependencies respect the layered architecture.

According to ARCHITECTURE.md:
Strict Rule: Higher-numbered layers may depend on any lower-numbered layer (and same-layer).
             Lower-numbered layers must never depend on higher-numbered layers.

Layer rules:
  01-transport → 01 only
  02-runtime   → 01-02 only
  03-driver    → 01-03 only
  04-services  → 01-04 only
  05-app-loop  → 01-05 only
  06-apps      → 01-06 (any lower layer)
  99-tests     → any
"""

import re
import sys
from pathlib import Path
from typing import Dict, Set, Tuple


def extract_layer_number(path: Path) -> int | None:
    """Extract layer number from crate path like crates/01-transport/foo."""
    parts = path.parts
    for part in parts:
        match = re.match(r"^(\d+)-", part)
        if match:
            return int(match.group(1))
    # testdata is not part of the layered architecture
    if "testdata" in parts:
        return None
    return None


def parse_cargo_toml(toml_path: Path) -> Tuple[str, Set[str]]:
    """Parse Cargo.toml and extract crate name and path dependencies."""
    with open(toml_path, "r") as f:
        content = f.read()

    # Extract package name
    name_match = re.search(
        r'^\[package\]\s*\nname\s*=\s*"([^"]+)"', content, re.MULTILINE
    )
    crate_name = name_match.group(1) if name_match else toml_path.parent.name

    # Extract path dependencies
    path_deps = set()
    # Match: dep-name = { path = "../../some/path" }
    for match in re.finditer(
        r'^\s*([a-zA-Z0-9_-]+)\s*=\s*\{[^}]*path\s*=\s*"([^"]+)"', content, re.MULTILINE
    ):
        dep_name = match.group(1)
        dep_path = match.group(2)
        path_deps.add((dep_name, dep_path))

    return crate_name, path_deps


def main():
    repo_root = Path(__file__).parent.parent
    crates_dir = repo_root / "crates"

    violations = []

    # Find all Cargo.toml files in the crates directory
    for cargo_toml in crates_dir.rglob("Cargo.toml"):
        # Skip target directories
        if "target" in cargo_toml.parts:
            continue

        crate_layer = extract_layer_number(cargo_toml)
        if crate_layer is None:
            continue  # Skip non-layered crates like testdata

        crate_name, deps = parse_cargo_toml(cargo_toml)

        for dep_name, dep_path in deps:
            # Resolve the dependency path relative to the crate
            dep_full_path = (cargo_toml.parent / dep_path).resolve()
            dep_layer = extract_layer_number(dep_full_path)

            if dep_layer is None:
                continue  # External or non-layered dependency

            # Same-layer dependencies are always allowed
            if dep_layer == crate_layer:
                continue

            # STRICT RULE: Higher layers can only depend on lower layers
            # (never the reverse)
            if dep_layer > crate_layer:
                violations.append(
                    {
                        "crate": crate_name,
                        "crate_path": str(cargo_toml.parent.relative_to(repo_root)),
                        "crate_layer": crate_layer,
                        "dep": dep_name,
                        "dep_path": str(dep_full_path.relative_to(repo_root)),
                        "dep_layer": dep_layer,
                    }
                )

    if violations:
        print("❌ Layer dependency violations found:\n")
        for v in violations:
            print(f"  {v['crate_path']}/")
            print(
                f"    Layer {v['crate_layer']:02d} crate '{v['crate']}' depends on layer {v['dep_layer']:02d} crate '{v['dep']}'"
            )
            print(f"    VIOLATION: Lower layer depends on higher layer!")
            print(f"    Dependency path: {v['dep_path']}")
            print()

        print(f"Found {len(violations)} violation(s)")
        print("\nStrict rule: Higher layers may only depend on lower layers.")
        print("Layer rules:")
        print("  01-transport → 01 only")
        print("  02-runtime   → 01-02 only")
        print("  03-driver    → 01-03 only")
        print("  04-services  → 01-04 only")
        print("  05-app-loop  → 01-05 only")
        print("  06-apps      → 01-06 (any lower layer)")
        print("  99-tests     → any")
        sys.exit(1)
    else:
        print("✅ All layer dependencies respect the strict rule")
        sys.exit(0)


if __name__ == "__main__":
    main()
