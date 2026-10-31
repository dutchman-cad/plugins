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
| Print | 205,364 | 0.20 |

All four importers and both panels fit the 16,777,216-byte cap. CI runs the host's unit tests
on Linux, macOS and Windows, plus Linux smoke tests and host/guest lint checks.
Earlier live checks, against the official SolidCraft 0.22.1 Linux release: the Fusion importer converts a synthetic cube and SolidCraft opens it, Pilot
answers "round all edges 3 mm" with the fillet command, and the hello panel's button
creates a box (3 of 3 pass). SPEC-NOTES.md records where the draft interface was
unclear and what this host chose.

Building from source requires sibling checkouts of [dutchman-bridge](https://github.com/dutchman-cad/bridge) and
[dutchman](https://github.com/dutchman-cad/dutchman). Pilot's portable core comes from
[dutchman-cad/pilot](https://github.com/dutchman-cad/pilot) at
`c496904e0a681aa19f84353f889f6aacaa594d86` (v0.1.0-beta.8), for both the host and Pilot plugin. Rust must support Wasmtime 50
and have the wasm32-wasip2 target installed.

## Sibling revisions

CI pins the sibling checkouts in `.github/workflows/ci.yml`, under the workflow
`env:` keys `BRIDGE_REV`, `DUTCHMAN_REV`, and `PRINT_REV`. To move a revision,
update its commit SHA and run CI. The checkout directories are `../dutchman-bridge`,
`../dutchman`, and `../print`, beside this repository.

## Download and run

Download a ready-to-run package from the CI run's **Artifacts** section. Choose
`dutchman-plugins-<OS>-<architecture>` for your system (for example Linux-X64,
macOS-ARM64 or Windows-X64). Packages are kept for 14 days. Unzip the download,
then unpack the `.tar.gz` inside it. Keep both executables and `examples/` together.
No Rust installation or sibling checkouts are needed to use a package.

From the unpacked directory:

- **Linux:** extract with `tar -xzf dutchman-plugins-Linux-X64.tar.gz`, then run
  `./dutchman-plugins list` and `./dutchman-plugins --yes panel print`.
- **macOS:** extract the archive with Finder or `tar -xzf dutchman-plugins-macOS-ARM64.tar.gz`,
  then run `./dutchman-plugins list` and `./dutchman-plugins --yes panel print`.
  Use the architecture in your artifact's name. macOS may require approval in
  System Settings → Privacy & Security for each downloaded executable.
- **Windows (PowerShell):** extract with `tar -xzf dutchman-plugins-Windows-X64.tar.gz`,
  then run `.\dutchman-plugins.exe list` and `.\dutchman-plugins.exe --yes panel print`.

The Print panel runs in the terminal; type `quit` to close it. Without a configured
companion it shows a setup message. To try the complete panel with a simulated
printer, save this as `config.toml` in the unpacked directory:

```toml
[backend]
kind = "simulated"

[[printers]]
id = "workbench"
name = "Workbench demo"
model = "Example printer"
```

In another terminal, start the bundled companion, then open the connected panel:

```sh
./dutchman-plugins companion -- --config config.toml serve --listen 127.0.0.1:8730 --data-dir private-jobs
./dutchman-plugins --yes --local dutchman-print=http://127.0.0.1:8730 panel print
```

On Windows use `.\dutchman-plugins.exe` in place of `./dutchman-plugins`.
Stop the companion with Ctrl+C. This example uses only the built-in simulated
printer. To prepare a selection, add `--selection-3mf path/to/selection.3mf` to the
panel command. Physical actions still ask for separate terminal confirmation.

The `companion` and `solidcraft` commands find executables beside the host, then
on `PATH`; `solidcraft` also checks standard macOS application and Windows
installation locations.
Set `DUTCHMAN_PRINT_BIN` or `SOLIDCRAFT_BIN` to an executable path to override
lookup. Arguments after `--` pass directly to that program, including paths with
spaces when quoted. For example, `./dutchman-plugins solidcraft -- --control 7878`
starts an installed SolidCraft with its control channel enabled. SolidCraft is a
separate download; it is not needed for the Print panel.

## Building and testing

The Python smoke tests are Linux-only. Host unit tests run on all three operating
systems in CI, and `tests/package_session.py` drives each package through a real terminal
session against the simulated printer.

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

`python3 tests/print_e2e.py` runs additional smoke tests against the real
Dutchman Print companion pinned by CI at `PRINT_REV=a47a6bac2ebbac9905fa61838fd98f41dcae6870`.
It prepares a synthetic mesh with the built-in simulated backend (no slicer needed),
checks separate Bed is clear and Print challenges, verifies printing, then uses a
printer-level Stop challenge and checks cancellation and replay refusal. It also checks
queue add/list/move/remove, confirmed dispatch and recent history. It uses only
127.0.0.1, starts and shuts down its own companion, and requires no printer. CI builds
`dutchman-print` alongside the host; local runs can set `DUTCHMAN_PRINT_BIN` and
`DUTCHMAN_PLUGINS_BIN` to their release binaries. Set `TMPDIR` to a writable build
scratch directory; binary defaults are under `${CARGO_TARGET_DIR:-./target}/release`.

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
printf '%s\n' 'Size=12' 'Make box' quit | $BIN --yes panel hello-panel
# Equivalent panel entry:
$BIN run hello-panel
```

A plugin argument can be an example ID or a directory. Example IDs are looked up in
`DUTCHMAN_PLUGINS_EXAMPLES` if set, else in an `examples` folder beside the binary or two
levels up from it (as in a `target/release` build), then the original source tree. Command parameters accept
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
Print, Pause, Resume, Stop, Cancel and Bed is clear. Machine buttons appear only for
actions listed in the selected printer’s `controls`; an empty list shows a monitoring-only
message, and older companions without `controls` keep the previous behavior.
Printer rows show state, and progress and time remaining when the companion reports them.
The slicing estimate reads as time, filament and layers, queue rows name printers rather
than showing ids, and times in Recent prints are shown to the second in UTC. The toolbar widget uses the
panel's status item. It refreshes every 5 seconds while a printer has a live job and
every 30 seconds otherwise.
A printer in `attention` (waiting for a person) or `busy` (a non-print operation is
running) shows the companion's reason under its row. Attention also marks the widget.
Stop ends whatever job is live, including one started outside Print, and is the only
action offered in attention. Cancel withdraws a prepared job that has not started.

The panel opens with a Farm overview: a summary line (printer count, how many are
printing, idle, need a bed check (every printer reporting `needs_clearing`, whatever its state), or are offline or in error, and the queue length) and one
line per printer with its state, job name and progress while printing, "needs bed check"
when the companion reports `needs_clearing`, and the queue entry proposed for it. Each line
has a Select button that makes that printer the panel's selected printer. The overview only
reads; it never sends a machine action, and every action stays in the reviewed flows below.
Printers are paged in groups of 20.

The panel's Farm queue adds a prepared job for the selected printer or any compatible
printer. Rows show FIFO position, job name, target, proposed printer and refusal reasons,
with Remove and Move up controls. Proposals are advisory and start nothing. Dispatch
appears only with a proposed printer and shows a fresh Start summary for personal
approval, using the same physical confirmation flow as Print. Bed clearance needs its
own separate approval. Each challenge is sent once; dispatch is never retried. Failed
or attempted entries keep their printer reserved until removed: check the tracked job
and printer first. Removing membership does not cancel a print or delete its job record.
The queue is paged in groups of 20. Recent prints shows the selected printer's
newest five history records with job name, outcome, ended time and duration (including
bed-clear audit records, whose duration is unknown).

The read entries `queue`, `queue-add` (`job`, `target`), `queue-remove` (`queue`) and
`queue-move` (`queue`, zero-based `position`) also expose queue management.
For dispatch, call `review` with `action: "dispatch"`, `queue` and the proposed `printer`.
Then call the physical `dispatch` entry with that queue id, the returned printer and
bound job id, challenge `confirmation` and verbatim `summary`.

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

Panels display headings, text, aligned rows, status percentages and numbered controls:

```text
== Hello — 0 boxes ==
[1] Size (id=size) = 10 mm [number; 1..1000; step 1]
[2] Make box (id=make-box) — button
```

Enter `2`, `make-box`, or `Make box` to press the button. Set an input with
`1=12` or `Size=12`. Choices accept an option name or a number starting at 1,
for example `Printer=2`. Checkboxes accept `true`/`false`, or toggle when selected
without a value. Text accepts plain text or a quoted JSON string for line breaks.
Disabled buttons cannot be selected. Numbers and control names refer to the latest
panel display; labels must be unique. Consecutive rows form aligned tables; text
line breaks and list prefixes are preserved.

JSON events remain supported, for example `{"id":"slice"}` then `{"id":"print"}`.
JSON choice indices still start at zero: `{"id":"status","inputs":{"printer":0}}`.
Numbered and named actions follow the same confirmation and policy checks as JSON events.
`hide` hides both surfaces and drops the timer; `show` starts a fresh interval.
`quit` stops the plugin. `widget` leaves only the widget visible; `open-panel` opens its panel.
Each machine action has two steps. Call the `review` read entry with `action`
(`print`, `cancel`, `pause`, `resume`, `clear` or `stop`), `printer`, and `job` for Print
and Cancel. It posts to `/api/v1/confirmations` (mapping `print` to API action
`start`) and stores and returns the server challenge. The physical entry then takes `printer`, `job` when needed,
`confirmation` (the challenge ID), and `summary` (the verbatim server sentence).
The panel fetches these challenges during rendering and shows each summary before
the button is pressed. Bed is clear has its own challenge and approval; Print
gets a separate challenge afterward. Expired challenges refresh on rendering;
the plugin requests wall-clock permission to check `expires_at`.
Physical actions display the plugin, machine, action and frozen parameters and
require typing `y` in the terminal. Piped or headless calls are refused even with
`--yes`. Only tests use `--confirm-physical`, with `DUTCHMAN_PLUGINS_TEST=1` and
127.0.0.1 services. The host attaches a fresh confirmation header; the component
never receives it. The physical request sends the challenge ID once. A 409
refreshes the challenge and asks for another press and approval; no machine
request is retried automatically. A failed machine action requires another confirmation.

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
