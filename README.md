# Dutchman Plugin Host

A standalone Rust/Wasmtime 50 component host for the proposed
SolidCraft plugin WIT. It drives stock SolidCraft through its documented,
authenticated loopback control channel, using solidcraft-bridge by path.
It links no SolidCraft crates and requires no SolidCraft modifications.

The examples contain Pilot, hello-panel, Print, and four independent geometry importers:
`importer-fusion` (.f3d/.f3z), `importer-inventor` (.ipt/.iam),
`importer-solidworks` (.sldprt), and `importer-freecad` (.FCStd).
Each importer links its own decoder plus the shared STEP writer and adapter.
Every component must fit the unchanged 16 MiB cap before the build script installs it.

Size-optimized release components (`wasm32-wasip2`):

| Importer | Bytes | MiB |
| --- | ---: | ---: |
| Fusion | 13,639,249 | 13.01 |
| Inventor | 4,938,375 | 4.71 |
| SolidWorks | 7,850,338 | 7.49 |
| FreeCAD | 5,302,767 | 5.06 |

Panel components:

| Panel | Bytes | MiB |
| --- | ---: | ---: |
| Hello panel | 175,846 | 0.17 |
| Print | 190,106 | 0.18 |

All four importers and both panels fit the 16,777,216-byte cap. Release host tests (24), smoke tests (16),
and host/guest lint checks pass. Earlier live checks, against the official SolidCraft 0.22.1 Linux
release: the Fusion importer converts a synthetic cube and SolidCraft opens it, Pilot
answers "round all edges 3 mm" with the fillet command, and the hello panel's button
creates a box (3 of 3 pass). SPEC-NOTES.md records where the draft interface was
unclear and what this host chose.

Requires sibling checkouts of [dutchman-bridge](https://github.com/dutchman-cad/bridge) and
[dutchman](https://github.com/dutchman-cad/dutchman). Pilot's portable core comes from
[dutchman-cad/pilot](https://github.com/dutchman-cad/pilot) at
`6933f2c7dccbb29a53ff0a10f81147195cdb1591` (v0.1.0-beta.5), for both the host and Pilot plugin. Rust must support Wasmtime 50
and have the wasm32-wasip2 target installed.

## Sibling revisions

CI pins the sibling checkouts in `.github/workflows/ci.yml`, under the workflow
`env:` keys `BRIDGE_REV` and `DUTCHMAN_REV`. To move either revision, update its
commit SHA and run CI. The checkout directories are `../dutchman-bridge` and
`../dutchman`, beside this repository.

## Building and testing

Build through the `plugins` build queue. It selects the target directory and
limits the job count. Set a writable scratch directory for temporary files:

```sh
scratch_dir="$PWD/.build-tmp"
mkdir -p "$scratch_dir"
buildq run plugins "$PWD" -- cargo --config "env.TMPDIR=\"$scratch_dir\"" build --release
TMPDIR="$scratch_dir" ./scripts/build-plugins.sh
buildq run plugins "$PWD" -- cargo --config "env.TMPDIR=\"$scratch_dir\"" fmt --all --check
buildq run plugins "$PWD" -- cargo --config "env.TMPDIR=\"$scratch_dir\"" clippy --release --all-targets -- -D warnings
buildq run plugins "$PWD" -- cargo --config "env.TMPDIR=\"$scratch_dir\"" test --release
TMPDIR="$scratch_dir" python3 tests/smoke.py
```

`cargo build --release --target wasm32-wasip2 -p <plugin-package>` produces the components directly:
neither cargo-component nor wasm-tools adaptation is needed. The build script
copies them beside their example manifests. The WIT includes the proposed physical actions, local services, mesh bodies,
status widgets and timed refresh additions, including
its unstable file-formats annotations; guest bindings enable that WIT feature.
Host and plugins share no private interfaces.

In CI, or when `buildq` is unavailable, the build script uses plain Cargo and
reads components from `CARGO_TARGET_DIR` (default `./target`), with Cargo's default
job count. Local queued builds continue to use three jobs.

Use the default ~/.cargo cache; do not create a second CARGO_HOME.
The build queue sets the target directory and a conservative job count.
Keep the combined job count at six or fewer when building concurrently; the
plugin build script uses three jobs. Keep the target directory below
8 GiB. Use release builds only, including tests
and lint checks; do not build debug artifacts.
Only core/format crates are built, never Dutchman's GUI. Avoid cleaning
other users' shared targets; this target belongs to this reference host.

```sh
BIN="<queue target>/release/dutchman-plugins"
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
prints permissions and asks y/N. --yes approves the current capabilities.
Physical machine actions still require their own terminal confirmation. Approvals and permission-gated stores live under
the platform data directory (override DUTCHMAN_PLUGINS_DATA for isolated tests).
Changed capabilities require new approval; code-only updates keep approval.

Start the existing stock application with `solidcraft --control 7878`, then use
`--port 7878` or SOLIDCRAFT_CONTROL_PORT. Without a port Bridge selects the sole
discovered token endpoint. SOLIDCRAFT_CONTROL_DIR and SOLIDCRAFT_CONTROL_TOKEN
work as in Bridge. No SolidCraft is needed for Pilot, check, or conversion without
--open. Live tests are in tests/live; see its README.

Network access is limited to approved local services with an address supplied by
`--local name=http://host:port`. Public `permissions.net` remains unsupported.
The host resolves and checks every local request, pins the checked addresses,
disables proxies and follows no redirects. WASI has no preopened files,
environment, arguments, sockets, or real wall clock by default.
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

## Print plugin

`plugins/print` prepares the selected bodies through a local Print job API v1
companion, such as [Dutchman Print](https://github.com/dutchman-cad/dutchman/blob/main/docs/print-api.md).
The panel offers printer choice, optional profile overrides, slicing estimates,
Print, Pause, Resume, Cancel and Bed is clear. Printer rows show state, progress
and time remaining. The toolbar widget uses the panel's status item. It refreshes
every 5 seconds while a printer is busy and every 30 seconds otherwise.

Set the companion address in this host's command line. It never comes from the
plugin. Until set, the host returns a named setup error. Only loopback, private,
link-local and `.local` names resolving entirely to those ranges are allowed;
HTTPS verifies the configured name. The port in the manifest is a hint.

```sh
$BIN --yes --local dutchman-print=http://127.0.0.1:8730 \
  --selection-3mf /path/to/selection.3mf panel print
```

This standalone host uses `--selection-3mf` as its selection. Supply a 3MF exported
from the bodies to print; the host reads and sends it directly, without exposing
bytes or paths to the component. Standard quality and the whole file are supported.
Named bodies and other quality requests are refused. Uploads are capped at 256 MiB.

Panel input is JSON, for example `{"id":"slice"}` then `{"id":"print"}`.
Choice inputs use an index: `{"id":"status","inputs":{"printer":0}}`.
`hide` hides both surfaces and drops the timer; `show` starts a fresh interval.
`quit` stops the plugin. `widget` leaves only the widget visible; `open-panel` opens its panel.
Physical CLI calls require `printer`, plus `job` for Print and Cancel.
Physical actions display the plugin, machine, action and frozen parameters and
require typing `y` in the terminal. Piped or headless calls are refused even with
`--yes`. Only tests use `--confirm-physical`, with `DUTCHMAN_PLUGINS_TEST=1` and
127.0.0.1 services. The host attaches a fresh confirmation header; the component
never receives it. A failed machine action requires another confirmation.

A companion requiring a password can declare a local secret in the manifest and
receive its value through `--local-secret name=value`. This reference host keeps
those values in memory for the process. It redacts exact echoed secrets and
confirmation values from response headers and bodies.

`python3 tests/smoke.py` includes a fake Print API on 127.0.0.1, with no companion
checkout, SolidCraft or printer required. Run it with `TMPDIR` set to a writable
build scratch directory. If the shell cannot create loopback sockets, run the same
suite through the queue after building the examples:

```sh
buildq run plugins "$PWD" -- cargo --config "env.TMPDIR=\"$scratch_dir\"" test --release --test smoke -- --ignored --nocapture
```

The Cargo smoke launcher is ignored in ordinary unit-test runs. `tests/live` is
separate.
