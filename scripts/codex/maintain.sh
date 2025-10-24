#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [ -f package-lock.json ]; then
  npm ci --prefer-offline --no-audit --fund=false
fi
cargo fetch || true
