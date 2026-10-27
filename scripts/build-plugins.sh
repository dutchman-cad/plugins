#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
target_dir="$(python3 - <<'PYTARGET'
import hashlib
from pathlib import Path
root = Path.cwd().resolve()
print(Path.home() / ".cache/buildq/target/plugins" / f"{root.name}-{hashlib.sha1(str(root).encode()).hexdigest()[:8]}")
PYTARGET
)"
plugins=(pilot hello-panel print importer-fusion importer-inventor importer-solidworks importer-freecad)
# Each cdylib has its own decoder dependency; the shared adapter uses static dispatch.
packages=()
for plugin in "${plugins[@]}"; do
  packages+=(-p "dutchman-$plugin-plugin")
done
scratch_dir="${TMPDIR:-$PWD/.build-tmp}"
mkdir -p "$scratch_dir"
buildq run plugins "$PWD" -- cargo --config "env.TMPDIR=\"$scratch_dir\"" build --release --target wasm32-wasip2 -j 3 "${packages[@]}"
# Validate every result before replacing any installed example component.
python3 - "$target_dir" <<'PY'
import sys
from pathlib import Path

target = Path(sys.argv[1])
oversized = []
for name in ("pilot", "hello_panel", "print", "importer_fusion", "importer_inventor", "importer_solidworks", "importer_freecad"):
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
  install -m 644 "$target_dir/wasm32-wasip2/release/$name" "examples/$plugin/${plugin//-/_}.wasm"
done
