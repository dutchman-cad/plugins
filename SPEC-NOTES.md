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
2. **6.1 — Approval hashes conflict with the requested reference-host behavior.**
   The draft hashes permissions and explicitly allows code-only updates without
   renewed approval; this reference host keys approval to component hashes. We require both the
   SHA-256 component hash and the complete validated manifest to remain identical.
   --yes explicitly authorizes the current version. check typechecks without running
   guest code; explicit check permits compilation without granting runtime approval.
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
