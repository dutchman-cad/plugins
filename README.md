# Dutchman Plugin Host

A standalone Rust/Wasmtime 50 component host for the unchanged proposed
SolidCraft plugin WIT. It drives stock SolidCraft through its documented,
authenticated loopback control channel, using solidcraft-bridge by path.
It links no SolidCraft crates and requires no SolidCraft modifications.

The examples contain Pilot, hello-panel, and four independent geometry importers:
`importer-fusion` (.f3d/.f3z), `importer-inventor` (.ipt/.iam),
`importer-solidworks` (.sldprt), and `importer-freecad` (.FCStd).
Each importer links its own decoder plus the shared STEP writer and adapter.
Every component must fit the unchanged 16 MiB cap before the build script installs it.

Size-optimized release components (`wasm32-wasip2`):

| Importer | Bytes | MiB |
| --- | ---: | ---: |
| Fusion | 13,523,761 | 12.90 |
| Inventor | 4,793,322 | 4.57 |
| SolidWorks | 7,427,095 | 7.08 |
| FreeCAD | 5,264,459 | 5.02 |

All four fit the 16,777,216-byte cap. Release host tests (15), smoke tests (8),
and host/guest lint checks pass. Live, against the official SolidCraft 0.22.1 Linux
release: the Fusion importer converts a synthetic cube and SolidCraft opens it, Pilot
answers "round all edges 3 mm" with the fillet command, and the hello panel's button
creates a box (3 of 3 pass). SPEC-NOTES.md records where the draft interface was
unclear and what this host chose.

Requires sibling checkouts of [dutchman-bridge](https://github.com/dutchman-cad/bridge) and
[dutchman](https://github.com/dutchman-cad/dutchman). Pilot's portable core comes from
[dutchman-cad/pilot](https://github.com/dutchman-cad/pilot) at 6933f2c. Rust must support Wasmtime 50
and have the wasm32-wasip2 target installed.

Build with at most six jobs, at reduced priority:

```sh
export CARGO_TARGET_DIR="$PWD/target"   # or any build folder
nice cargo build --release -j 3
./scripts/build-plugins.sh
nice cargo fmt --all --check
nice cargo clippy --release --all-targets -- -D warnings
nice cargo test --release
python3 tests/smoke.py
```

`cargo build --release --target wasm32-wasip2 -p <plugin-package>` produces the components directly:
neither cargo-component nor wasm-tools adaptation is needed. The build script
copies them beside their example manifests. The WIT remains unchanged, including
its unstable file-formats annotations; guest bindings enable that WIT feature.
Host and plugins share no private interfaces.

Use the default ~/.cargo cache; do not create a second CARGO_HOME.
The target directory and a conservative job default are set in .cargo/config.toml.
Keep the combined job count at six or fewer when building concurrently; the
plugin build script uses three jobs. Keep the target directory below
8 GiB (`du -sh "$CARGO_TARGET_DIR"`). Use release builds only, including tests
and lint checks; do not build debug artifacts.
Only core/format crates are built, never Dutchman's GUI. Avoid cleaning
other users' shared targets; this target belongs to this reference host.

```sh
BIN="$CARGO_TARGET_DIR/release/dutchman-plugins"
$BIN list
$BIN check examples/pilot
$BIN run pilot how 'round all edges 3 mm'
$BIN run pilot plan 'make a 20 mm cube'
$BIN import importer-fusion examples/generated_unit_cube.f3d --open
printf '%s\n' '{"id":"make-box","inputs":{"size":12}}' quit | $BIN --yes panel hello-panel
# Equivalent panel entry:
$BIN run hello-panel
```

A plugin argument can be an example ID or a directory. Command parameters accept
JSON, or plain text which the CLI wraps as a JSON string. Output files use the
input stem and .solidcraft extension; existing outputs are refused. First use
prints permissions and asks y/N. --yes approves the exact current component and
manifest, for unattended tests. Approvals and permission-gated stores live under
the platform data directory (override DUTCHMAN_PLUGINS_DATA for isolated tests).
Changed bytes or manifest require new approval.

Start the existing stock application with `solidcraft --control 7878`, then use
`--port 7878` or SOLIDCRAFT_CONTROL_PORT. Without a port Bridge selects the sole
discovered token endpoint. SOLIDCRAFT_CONTROL_DIR and SOLIDCRAFT_CONTROL_TOKEN
work as in Bridge. No SolidCraft is needed for Pilot, check, or conversion without
--open. Live tests are in tests/live; see its README.

There is **no network import implementation** in this version. net.fetch always
returns denied; manifests requesting network access are refused. WASI has no
preopened files, environment, arguments, sockets, or real wall clock by default.
No adjacent input-folder access is granted. The per-family importers support
Fusion, Inventor, SolidWorks and FreeCAD, via Dutchman's clean-room codecs,
and returns conversion losses as text. The shared adapter emits geometry-only
designs; it does not reconstruct history or external assemblies.

The host enforces total guest linear memory, instance/table limits, stack size,
epoch-based asynchronous deadlines including instantiation, 8 MiB lifting/copy
fuel, JSON depth/size, logs, storage, streams and panel output limits. Every
command/event starts fresh; a trap discards the instance. Persistent state needs
the explicit store permission. Pure Pilot has no permissions.

The stock control protocol cannot provide the full draft's atomic multi-command
transactions or remove its implicit recovery/design-folder grants. These are
recorded with the other implementation choices in [SPEC-NOTES.md](SPEC-NOTES.md).
Do not rely on this host to roll back earlier engine edits after guest failure.
The demonstration box entry has exactly one editing command, hence one undo step.

Original code is MIT OR Apache-2.0. See [ATTRIBUTION.md](ATTRIBUTION.md) for the
Apache-2.0 WIT/spec, Dutchman adapter, Pilot and synthetic fixture provenance.
