#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
plugins=(pilot hello-panel importer-fusion importer-inventor importer-solidworks importer-freecad)
# Each cdylib has its own decoder dependency; the shared adapter uses static dispatch.
packages=()
for plugin in "${plugins[@]}"; do
  packages+=(-p "dutchman-$plugin-plugin")
done
nice cargo build --release --target wasm32-wasip2 -j 3 "${packages[@]}"
# Validate every result before replacing any installed example component.
python3 - <<'PY'
import os
from pathlib import Path

target = Path(os.environ["CARGO_TARGET_DIR"])
oversized = []
for name in ("pilot", "hello_panel", "importer_fusion", "importer_inventor", "importer_solidworks", "importer_freecad"):
    path = target / "wasm32-wasip2/release" / f"dutchman_{name}_plugin.wasm"
    size = path.stat().st_size
    print(f"{path.name}: {size:,} bytes", flush=True)
    if size > 16 * 1024 * 1024:
        oversized.append(path.name)
if oversized:
    raise SystemExit(f"Exceeds the unchanged 16 MiB cap: {', '.join(oversized)}")
PY
for plugin in "${plugins[@]}"; do
  name="dutchman_${plugin//-/_}_plugin.wasm"
  install -m 644 "$CARGO_TARGET_DIR/wasm32-wasip2/release/$name" "examples/$plugin/${plugin//-/_}.wasm"
done
