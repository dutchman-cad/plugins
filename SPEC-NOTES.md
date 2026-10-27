# Draft interface feedback

Each entry records the section, question, and this reference host's choice.

1. **6.3 — Can an external host provide a transaction spanning callbacks?**
   Stock docs/control-protocol.md offers engine.execute and batch rollback, but no
   session lock, transaction begin/commit, or one-step undo grouping for arbitrary
   guest callbacks. We forward calls immediately through engine.execute, preserving
   real command results. Each edit is its own undo step. A subsequent guest error,
   trap or timeout cannot roll back earlier commands. We do not emulate rollback
   with undo, which could undo another client's edits. Hello-panel issues one box
   command, so its transaction is one undo step. Full atomic multi-command transactions
   need a stock control-protocol extension; this is a material compatibility limitation.
   Bridge's blocking control I/O runs on a detached worker so dropping a guest call
   does not wait for a socket timeout during CLI shutdown. Dispatch checks the
   deadline after authentication. A command already sent to stock SolidCraft may
   finish after the guest deadline, as the control protocol documents.
   Compilation also uses a detached worker, so its 60-second timeout does not
   leave the CLI waiting for Tokio's blocking pool to finish the compile.
2. **6.1 — Approval fingerprints.**
   Approval hashes the validated capabilities: commands, permissions, limits,
   world, entries and confirmations, surfaces, importer formats and folder access.
   Code-only updates keep approval. User addresses are separate process settings.
   The component SHA-256 is displayed for inspection. check typechecks without
   running guest code or granting runtime approval.
3. **9a / Dutchman adapter — How can a filesystem-oriented decoder use streams?**
   Dutchman's public read entry point takes a native path. We copied only its private
   byte decoder locally and removed filesystem detection; conversion receives the WIT
   stream bytes. The original core model/report types remain a path dependency. Local copies of the geometry container/document and guarded
   STEP helpers avoid linking the full history/subprocess writer and general encoder
   registry. Typed helix inspection replaces whole-model JSON serialization. Codec dispatch uses the declared family
   extensions; each selected codec validates its own payload. The importer buffers
   decoder input inside guest memory, emits in
   1 MiB pieces, uses import-only geometry, disables subprocess verification and history
   replay, and has no wall-clock calls. It requests geometry decode depth directly
   instead of decoding unused editable history first; the loss report always states
   that history, sketches and parameters are omitted. Model validation and decoder
   panic guards remain. It does not resolve external assembly parts.
4. **5 / 9a — Is reads_input_folder mandatory for a first implementation?**
   This host refuses manifests requesting it. Only the main already-open regular file
   is readable; all nonempty input paths are denied. This avoids an incomplete symlink
   sandbox. Supporting assemblies requires handle-based folder containment first.
5. **5 — How does an offline host validate command IDs?**
   check validates against Pilot's bundled stock command catalogue with exact,
   case-sensitive comparison. Live control forwarding still reports missing/changed
   commands. A different stock release may require a refreshed catalogue snapshot.
6. **3 / 6.7 — How does the text panel retain state across fresh instances?**
   Each command and panel event starts a new store/instance. Successful handle/render/
   markers calls for a single event share an instance. The demonstration keeps size and
   count in the permission-gated store, rather than guest globals. Executed notifications
   cannot chain further executions in this text UI; design-changed is not subscribed.
7. **6.2 — Does copy fuel cover exported results too?**
   Wasmtime 50's component lifting uses hostcall fuel for both host-call arguments
   and lifted export results. We set 8 MiB, then validate JSON, panel and report caps.
   WASI stdout/stderr share a 200 KiB bounded pipe and are emitted with the
   plugin prefix, 200-line total budget and 1 KiB per-line cap when the instance drops.
8. **6.4 — Does an empty external PathScope forbid all engine file access?**
   Requests send allow_paths: [] at the outer envelope. Stock control additionally
   grants its recovery and eligible active-design folder per its documented policy;
   an external host cannot remove those grants. The deny list blocks the file/doc
   command families; metadata writes are namespaced and handler-safe IDs are fixed.
9. **6.1 — What about listing-only manifests and competing importer extensions?**
   list accepts listing-only manifests and rejects duplicate extensions among its
   enabled bundled examples. Explicit directory execution treats only that directory
   as enabled; no global installation registry or disabled-plugin UI is provided.
10. **4 — How does Wasmtime 50 gate unstable importer imports?**
    bindgen generates all interfaces, but also generates a LinkOptions parameter
    for this world. Linking requires LinkOptions::file_formats(true); the default
    leaves these imports unlinked. This is a runtime feature switch in addition
    to the guest binding generator's features: ["file-formats"].
11. **4 / Rust component bindings — Can all generated guest ABI glue forbid unsafe?**
    wit-bindgen's export! macro necessarily emits unsafe canonical ABI functions
    and export_name attributes. Handwritten plugin code denies unsafe; only a
    small component_exports module permits the generated trampolines. Both host
    targets forbid unsafe entirely. No handwritten unsafe code is introduced.
12. **6.2 — Can the all-formats converter fit the frozen component cap?**
    The first release build (opt-level 3, thin LTO) was 88,038,990 bytes
    (83.96 MiB), and the host correctly refuses it. The reference importer now
    selects four codecs directly, uses only the guarded STEP-restatement helpers,
    and adapts Dutchman's geometry-only container/document path locally.
    Release builds use opt-level z, one codegen unit and fat LTO for size.
    No cap is raised and no host-side decoding is introduced.
13. **6.2 / adapter size — Which generic helpers retain unrelated code?**
    Four direct decoders, geometry-only container helpers and size-optimized fat LTO
    reduced the first 88,038,990-byte build to 27,859,516 bytes (26.57 MiB), still
    above 16 MiB. The STEP wrapper now constructs StepCodec directly and uses typed
    helix inspection; visible FreeCAD bodies are filtered in place, keeping unused
    geometry rows rather than serializing/deserializing the full CADIR model.
    Existing references in product body lists are filtered with the bodies.
    Decoder calls also use static dispatch, so their unused detect/inspect methods
    are not retained through a Codec trait-object vtable.
    With these changes the cached-artifact link measured 21,014,632 bytes
    (20.04 MiB), still over the cap. Requesting geometry-only decode directly
    measured 21,015,270 bytes (20.04 MiB); it matches the adapter's geometry-only
    output and reports that choice, but did not shrink the linked decoder code.
    These latest measurements reuse compiled dependency artifacts because the
    default Cargo cache lacks dependencies and crates.io DNS is unavailable in
    this session. No Wasm size optimizer is installed. The component is not
    installed or runtime-verified: the frozen cap remains a blocking limit.
14. **5 — What bounds apply to listing metadata not frozen in the draft?**
    This host caps plugin.json at 16 KiB, uses a lowercase slug of at most 64
    bytes for id, and caps name/version/description at 80/32/1024 characters.
    Format labels use the entry-label limit of 80 characters. Component names
    must be plain, non-hidden filenames, and component symlinks are refused.
    These are local validation choices; listing-only manifests remain accepted
    by list and cannot execute.
15. **9a (Cost) / 6.2 — One importer component per format family.**
    This supersedes the all-formats size blocker in entries 12–13. Four separate
    cdylibs supply Fusion (.f3d/.f3z), Inventor (.ipt/.iam), SolidWorks (.sldprt)
    and FreeCAD (.FCStd) decoders. A shared generic adapter uses static dispatch;
    each leaf selects its decoder, extension mapping and WIT export. All share
    the geometry-only STEP writer and stream/container helpers. The shared
    adapter no longer consults Dutchman's all-format registry for codec availability.
    With the unchanged release profile (opt-level z, one codegen unit, fat LTO,
    stripped symbols), the components measure:

    | Component | Bytes | MiB |
    | --- | ---: | ---: |
    | Fusion | 13,523,761 | 12.90 |
    | Inventor | 4,793,322 | 4.57 |
    | SolidWorks | 7,427,095 | 7.08 |
    | FreeCAD | 5,264,459 | 5.02 |

    All are below the unchanged 16,777,216-byte cap; no oversized component
    requires a size-breakdown exception. The build script reports every size and
    checks all components before installing any example. Formats remain geometry
    only: embedded Fusion multi-document data is handled by its decoder; external
    assembly part files cannot be resolved through the granted main-file stream.
    The live Fusion test now writes the synthetic cube's .solidcraft output before
    attempting to open it. In this session, live open and panel checks are blocked
    by connection refusal at the discovered stock control endpoint; no SolidCraft
    executable is on PATH. This is separate from component size and conversion.

16. **6.4a — Physical actions in a standalone terminal.**
    Every physical entry asks on the host terminal before instantiation, including
    entries queued by request-execute. Approval never accepts a machine action.
    Headless calls are denied. The test switch additionally requires
    DUTCHMAN_PLUGINS_TEST=1 and 127.0.0.1 service origins. The prompt names the
    plugin, machine kind, fixed machine and action, and exact frozen JSON params.
    Calls are serialized by the text event loop; settings are immutable during
    a call. A 32-byte OS-random, lowercase hexadecimal token lives only in that
    call's State, expires after 60 seconds, and attaches only to local requests.
    Binding follows that State's plugin, entry and frozen params; there is no
    separate token registry or replay interface. The companion consumes tokens
    once, including failed actions. Header injection is refused case-insensitively.
    Exact echoes in response names, values and bytes are redacted before lifting.
    A changed approval record or selection-file size or modification time cancels
    the pending prompt. This host has no live design-change subscription or mutable
    settings UI; cancellation on external application edits is not implemented.
17. **6.5a — Local transport and settings.**
    --local name=origin supplies addresses outside the manifest. Ports are hints,
    never discovered or connected automatically. Literal loopback, RFC 1918, ULA,
    link-local and mapped IPv4 addresses are checked. Only .local host names may
    resolve; every answer must be local. Each request resolves afresh and uses a
    fresh reqwest client pinned to those answers, without proxies, pooling,
    cookies or redirects. HTTPS uses rustls verification with system trust roots. Public net remains
    denied. Limits are 64 requests per call, 1 MiB request bodies, 8 MiB responses,
    64 headers of up to 8 KiB and a 20-second HTTP deadline. Local secrets use
    manifest-declared headers and host-only --local-secret values, held in memory;
    no platform keychain adapter is provided. Forged secret headers are replaced.
18. **6.5a — Host-made mesh fallback.**
    --selection-3mf supplies an already exported selection. Bridge export covers
    the whole design. The documented file.save_mesh command defaults to all
    bodies and has no quality parameter; the documented inspection does not
    define selection-to-body mapping. This host uses the explicit file selection
    fallback. Only empty bodies and standard quality are accepted; named bodies
    and other qualities return invalid.
    Mesh bytes stay in the host, require mesh permission, and replace Content-Type
    with model/3mf. Nonempty plugin body is invalid, a second mesh is limit, and
    a file larger than 256 MiB fails before any connection. The host passes the
    file unchanged; the companion validates the 3MF package. This fallback does
    not claim per-quality equivalence to file.save_mesh.
19. **6.7a / 9b — Text surfaces and refresh.**
    The text host validates surface worlds, labels, entry references and duplicate
    widgets. It prints the widget from the same single validated status item as
    the panel, with the state enum standing for the host colour. Invalid renders
    keep the previous panel and widget. The menu offers open-panel; widget leaves
    only that surface visible. There is no desktop ribbon, core-slot registry or simultaneous multi-plugin widget layout;
    cross-plugin placement caps do not apply to this one-plugin text UI.
    The visible loop owns one timer; panel imports clamp intervals to 2 seconds,
    replace prior schedules and coalesce missed ticks. hide drops the pending
    tick and show starts a fresh interval. Actions and transactions cannot schedule.
    Successful panel calls still start fresh instances as described in entry 6;
    only the visible surface retains the interval. Dropping it clears the timer.
20. **WIT store.previous-version.**
    Version records are separate from plugin storage and readable without store
    permission. A successful command, import or panel event records the version;
    failed or trapped calls leave the previous record intact. The reference host
    does not implement schema generation, input/output schemas or plugin test from
    section 5a. Unknown unsupported manifest fields continue to be refused.
21. **Print API v1 — Fleet controls.**
    The component uses the additive clear route and needs_clearing field, handles
    204 responses, encodes opaque IDs and query values, and displays companion
    errors as plain text. Physical entries require explicit printer parameters;
    start and cancel also require a job. Panel clicks freeze these IDs before
    asking the host. Start also checks its prepared job matches that printer.
    Profile overrides are optional query fields. The companion owns bed clearance,
    fresh telemetry, prepared-file integrity and token reuse checks. No machine
    action is retried automatically.
22. **Build dependencies — cadmpeg revision.**
    Importer dependencies use fb1cd0a, matching the sibling Dutchman core. Mixed
    revisions produce incompatible CADIR and report types. The geometry-only
    adapter sets the core's new source_step field to None; its input families
    do not carry an unchanged source STEP file.
23. **6.1 — Compilation deadline.**
    Cranelift compiles inside a host-owned Rayon pool of three threads. Serial
    compilation of the current Fusion component exceeded 60 seconds in smoke.
    Parallel compilation keeps the existing 60-second deadline and 16 MiB cap;
    the compiler pool is bounded independently of Wasm execution.
