use crate::{
    bindings::{command as cb, importer as ib, panel as b},
    local::{Confirmation, FetchBudget, Refresh, Settings},
    manifest::{MIB, Manifest},
    policy, storage,
};
use anyhow::{Context, Result, ensure};
pub use b::exports::solidcraft::plugin::panel as panel_types;
use b::solidcraft::plugin::{engine, host, log, net, store, types};
use std::{
    collections::BTreeMap,
    io::{BufRead, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use types::PluginError as E;
use wasmtime::{
    Config, Engine, ResourceLimiter, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
pub struct EpochClock;
impl wasmtime_wasi::clocks::HostWallClock for EpochClock {
    fn resolution(&self) -> Duration {
        Duration::from_secs(1)
    }
    fn now(&self) -> Duration {
        Duration::ZERO
    }
}
pub struct Budget {
    pub used: usize,
    pub max: usize,
    tables: usize,
}
impl ResourceLimiter for Budget {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let delta = desired.saturating_sub(current);
        wasmtime::ensure!(
            maximum.is_none_or(|m| desired <= m) && self.used.saturating_add(delta) <= self.max,
            "limit: total linear memory"
        );
        self.used += delta;
        Ok(true)
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let delta = desired.saturating_sub(current);
        wasmtime::ensure!(
            maximum.is_none_or(|m| desired <= m) && self.tables.saturating_add(delta) <= 10000,
            "limit: table elements"
        );
        self.tables += delta;
        Ok(true)
    }
    fn memories(&self) -> usize {
        4
    }
    fn instances(&self) -> usize {
        16
    }
    fn tables(&self) -> usize {
        16
    }
}
pub struct State {
    settings: Settings,
    confirmation: Option<Confirmation>,
    fetch_budget: FetchBudget,
    panel_call: bool,
    read_only: bool,
    refresh: Option<u32>,
    wasi_output: wasmtime_wasi::p2::pipe::MemoryOutputPipe,
    deadline: std::time::Instant,
    pub manifest: Manifest,
    ctx: WasiCtx,
    table: ResourceTable,
    budget: Budget,
    pub transaction: bool,
    calls: usize,
    logs: usize,
    pub queued: Vec<(String, String)>,
    store_path: PathBuf,
    values: BTreeMap<String, Vec<u8>>,
    pub input: Option<std::fs::File>,
    read_bytes: u64,
    pub output: Vec<u8>,
    pub port: Option<u16>,
    session: Option<solidcraft_bridge::Session>,
}
impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}
impl types::Host for State {}
impl State {
    pub fn new(manifest: Manifest, port: Option<u16>) -> Result<Self> {
        let mut builder = WasiCtxBuilder::new();
        let wasi_output = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(200 * 1024);
        builder
            .stdout(wasi_output.clone())
            .stderr(wasi_output.clone());
        builder
            .allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        if !manifest.permissions.clock {
            builder.wall_clock(EpochClock);
        }
        let store_path = storage::data_dir()?
            .join("plugin-data")
            .join(format!("{}.json", manifest.id));
        let values = if manifest.permissions.store {
            storage::read(&store_path)?
        } else {
            BTreeMap::new()
        };
        let max = manifest.limits.memory_mib * MIB;
        Ok(Self {
            wasi_output,
            settings: Settings::default(),
            confirmation: None,
            fetch_budget: FetchBudget::default(),
            panel_call: false,
            read_only: false,
            refresh: None,
            deadline: std::time::Instant::now() + Duration::from_secs(30),
            manifest,
            ctx: builder.build(),
            table: ResourceTable::new(),
            budget: Budget {
                used: 0,
                max,
                tables: 0,
            },
            transaction: false,
            calls: 0,
            logs: 0,
            queued: vec![],
            store_path,
            values,
            input: None,
            read_bytes: 0,
            output: vec![],
            port,
            session: None,
        })
    }
    pub fn reset(&mut self, transaction: bool) {
        self.transaction = transaction;
        self.calls = 0;
        self.fetch_budget = FetchBudget::default();
        self.logs = 0;
        self.queued.clear();
    }
    fn store_allowed(&self) -> Result<(), E> {
        if self.manifest.permissions.store {
            Ok(())
        } else {
            Err(E::Denied("store permission required".into()))
        }
    }
}
// Uncancellable blocking work may finish after its caller times out. A detached
// thread lets the deadline return without Tokio shutdown waiting for that work.
async fn detached_work<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("plugin-worker".into())
        .spawn(move || {
            let _ = send.send(job());
        })?;
    receive.await.context("Control worker stopped")?
}
fn failed(e: impl std::fmt::Display) -> E {
    E::Failed(e.to_string())
}
fn invalid(e: impl std::fmt::Display) -> E {
    E::Invalid(e.to_string())
}
impl engine::Host for State {
    async fn run(&mut self, command: String, args: String) -> Result<String, E> {
        self.calls += 1;
        if self.calls > if self.transaction { 10000 } else { 1000 } {
            return Err(E::Limit("Engine call count".into()));
        }
        if self.read_only
            && ![
                "document.inspect",
                "document.metadata",
                "model.faces",
                "model.edges",
                "engine.describe",
            ]
            .contains(&command.as_str())
        {
            return Err(E::Denied("Read actions may only inspect".into()));
        }
        let args = policy::json(&args).map_err(invalid)?;
        policy::check(
            &self.manifest.id,
            &self.manifest.commands,
            self.transaction,
            &command,
            &args,
        )
        .map_err(|e| E::Denied(e.to_string()))?;
        let port = self.port;
        let prior = self.session.clone();
        let deadline = self.deadline;
        let (session, result) = detached_work(move || -> Result<_> {
            let session = match prior {
                Some(s) => s,
                None => solidcraft_bridge::Session::connect(
                    port,
                    std::env::var("SOLIDCRAFT_CONTROL_TOKEN").ok(),
                )?,
            };
            ensure!(
                std::time::Instant::now() < deadline,
                "limit: engine dispatch deadline"
            );
            // Path scope belongs to the outer control envelope, not command arguments.
            let value = session.call(
                "engine.execute",
                serde_json::json!({"command":command,"params":args,"allow_paths":[]}),
            )?;
            Ok((session, value.to_string()))
        })
        .await
        .map_err(failed)?;
        self.session = Some(session);
        policy::json(&result).map_err(invalid)?;
        Ok(result)
    }
}
impl log::Host for State {
    async fn log(&mut self, level: log::Level, message: String) {
        for line in message.lines().take(200usize.saturating_sub(self.logs)) {
            let line: String = line
                .chars()
                .scan(0, |n, c| {
                    *n += c.len_utf8();
                    (*n <= 1024).then_some(c)
                })
                .collect();
            eprintln!("[plugin {}] {level:?}: {line}", self.manifest.id);
            self.logs += 1;
        }
    }
}
impl store::Host for State {
    async fn previous_version(&mut self) -> Option<String> {
        storage::data_dir()
            .ok()
            .and_then(|p| std::fs::read_to_string(p.join("versions").join(&self.manifest.id)).ok())
    }
    async fn get(&mut self, key: String) -> Result<Option<Vec<u8>>, E> {
        self.store_allowed()?;
        if !storage::key(&key) {
            return Err(invalid("Invalid store key"));
        }
        Ok(self.values.get(&key).cloned())
    }
    async fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), E> {
        self.store_allowed()?;
        if !storage::key(&key) {
            return Err(invalid("Invalid store key"));
        }
        let mut next = self.values.clone();
        next.insert(key, value);
        storage::validate(&next).map_err(|e| E::Limit(e.to_string()))?;
        storage::write(&self.store_path, &next).map_err(failed)?;
        self.values = next;
        Ok(())
    }
    async fn remove(&mut self, key: String) -> Result<(), E> {
        self.store_allowed()?;
        if !storage::key(&key) {
            return Err(invalid("Invalid store key"));
        }
        let mut next = self.values.clone();
        next.remove(&key);
        storage::write(&self.store_path, &next).map_err(failed)?;
        self.values = next;
        Ok(())
    }
    async fn keys(&mut self) -> Result<Vec<String>, E> {
        self.store_allowed()?;
        Ok(self.values.keys().cloned().collect())
    }
}
impl net::Host for State {
    async fn fetch(&mut self, req: net::Request) -> Result<net::Response, E> {
        if self.transaction {
            return Err(E::Denied("Network in a transaction".into()));
        }
        tokio::time::timeout(
            Duration::from_secs(20),
            self.fetch_budget.fetch(
                &self.manifest,
                &self.settings,
                self.confirmation.as_ref(),
                req,
            ),
        )
        .await
        .unwrap_or_else(|_| Err(E::Limit("Local request exceeded 20 seconds".into())))
    }
    async fn has_secret(&mut self, name: String) -> bool {
        self.manifest
            .permissions
            .local
            .iter()
            .filter_map(|s| s.secret.as_ref())
            .any(|s| s.name == name)
            && self.settings.secrets.contains_key(&name)
    }
}
impl host::Host for State {
    async fn schedule_refresh(&mut self, seconds: u32) -> Result<(), E> {
        if !self.panel_call || self.transaction {
            return Err(E::Denied("Refresh only in panel calls".into()));
        }
        self.refresh = Some(seconds.max(2));
        Ok(())
    }
    async fn request_execute(&mut self, entry: String, params: String) -> Result<(), E> {
        if self.transaction || self.read_only {
            return Err(E::Denied("request-execute requires an edit handler".into()));
        }
        if !self
            .manifest
            .entries
            .iter()
            .any(|e| e.name == entry && (e.kind == "transaction" || e.effect == "physical"))
        {
            return Err(invalid("Unknown transaction or physical entry"));
        }
        policy::json(&params).map_err(invalid)?;
        if self.queued.len() >= 4 {
            return Err(E::Limit("At most four requested transactions".into()));
        }
        self.queued.push((entry, params));
        Ok(())
    }
}
impl ib::solidcraft::plugin::input::Host for State {
    async fn read(&mut self, path: String, offset: u64, len: u32) -> Result<Vec<u8>, E> {
        if !path.is_empty() {
            return Err(E::Denied("No input folder access".into()));
        }
        if len as usize > 8 * MIB || self.read_bytes.saturating_add(u64::from(len)) > (1 << 30) {
            return Err(E::Limit("Input stream cap".into()));
        }
        let f = self
            .input
            .as_mut()
            .ok_or_else(|| invalid("Input unavailable"))?;
        f.seek(SeekFrom::Start(offset)).map_err(failed)?;
        let mut bytes = vec![0; len as usize];
        let count = f.read(&mut bytes).map_err(failed)?;
        bytes.truncate(count);
        self.read_bytes += count as u64;
        Ok(bytes)
    }
    async fn size(&mut self, path: String) -> Result<Option<u64>, E> {
        if !path.is_empty() {
            return Ok(None);
        }
        self.input
            .as_ref()
            .map(|f| f.metadata().map(|m| m.len()).map_err(failed))
            .transpose()
    }
}
impl ib::solidcraft::plugin::output::Host for State {
    async fn write(&mut self, bytes: Vec<u8>) -> Result<(), E> {
        if bytes.len() > 8 * MIB || self.output.len().saturating_add(bytes.len()) > 256 * MIB {
            return Err(E::Limit("Output stream cap".into()));
        }
        self.output.extend(bytes);
        Ok(())
    }
}
pub struct Plugin {
    pub settings: Settings,
    version_dir: PathBuf,
    pub manifest: Manifest,
    pub engine: Engine,
    component: Component,
    stop: Arc<AtomicBool>,
    physical_pending: AtomicBool,
}
impl Drop for Plugin {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Plugin {
    pub async fn load(dir: &std::path::Path, yes: bool, check: bool) -> Result<Self> {
        let manifest = Manifest::read(dir)?;
        let filename = manifest
            .component
            .as_ref()
            .context("Listing-only plugin has no component")?;
        let path = dir.join(filename);
        ensure!(
            !std::fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Component symlinks refused"
        );
        ensure!(
            std::fs::metadata(&path)?.len() <= (manifest.limits.component_mib * MIB) as u64,
            "Component exceeds size limit"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take((manifest.limits.component_mib * MIB + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= manifest.limits.component_mib * MIB,
            "Component exceeds size limit"
        );
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let approval_path = storage::data_dir()?
            .join("approvals")
            .join(format!("{}.json", manifest.id));
        let fingerprint = crate::manifest::approval(&manifest);
        println!(
            "{} {} ({})\n{}\nWorld: {}",
            manifest.name,
            manifest.version,
            manifest.id,
            manifest.description,
            manifest.world.as_deref().unwrap_or("listing only")
        );
        if manifest.commands.is_empty() {
            println!("No engine commands requested.");
        } else {
            println!(
                "Can read and change open designs with: {}",
                manifest.commands.join(", ")
            );
        }
        if manifest.permissions.store {
            println!("Keeps up to 4 MiB on this computer.");
        }
        if manifest.permissions.clock {
            println!("Can read the wall clock.");
        }
        println!(
            "May use up to {} MiB of memory, {} KiB of Wasm stack, and {} seconds per change.",
            manifest.limits.memory_mib,
            manifest.limits.stack_kib,
            manifest.limits.transaction_seconds
        );
        if let Some(formats) = &manifest.formats {
            println!(
                "Opens files with these extensions: {}",
                formats
                    .iter()
                    .map(|f| format!("{} ({})", f.extension, f.label))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("Import deadline: 120 seconds; output cap: 256 MiB.");
        }
        for service in &manifest.permissions.local {
            println!(
                "Talks to a local service you set up: {} (default port {}).",
                service.name, service.port
            );
            if service.mesh {
                println!("Can send meshes of your design to {}.", service.name);
            }
        }
        for e in manifest.entries.iter().filter(|e| e.effect == "physical") {
            println!(
                "Can start and stop machines: {} ({}) {:?}.",
                e.label,
                e.machine_kind.as_deref().unwrap_or(""),
                e.confirm
            );
        }
        if manifest.surfaces.iter().any(|s| s.kind == "toolbar-widget") {
            println!("Shows a status area in the toolbar.");
        }
        if manifest.world.as_deref() == Some("panel-plugin") {
            println!(
                "Can refresh while its panel or status area is visible, at most once every 2 seconds."
            );
        }
        println!("Component SHA-256: {hash}");
        if !check && std::fs::read_to_string(&approval_path).ok().as_deref() != Some(&fingerprint) {
            if !yes {
                eprint!("Approve these permissions? y/N ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                ensure!(
                    answer.trim().eq_ignore_ascii_case("y"),
                    "Plugin was not approved"
                );
            }
            let approvals = approval_path.parent().context("Approval directory")?;
            std::fs::create_dir_all(approvals)?;
            if !approval_path.exists() {
                ensure!(
                    std::fs::read_dir(approvals)?.count() < 256,
                    "Approval registry exceeds 256 plugins"
                );
            }
            std::fs::write(approval_path, fingerprint)?;
        }
        let mut config = Config::new();
        config
            .epoch_interruption(true)
            .wasm_component_model(true)
            .max_wasm_stack(manifest.limits.stack_kib * 1024)
            .async_stack_size(manifest.limits.stack_kib * 1024 + 2 * MIB);
        let engine = Engine::new(&config)?;
        let compile_engine = engine.clone();
        let component = tokio::time::timeout(
            Duration::from_secs(60),
            detached_work(move || {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(3).build()?;
                pool.install(|| Ok(Component::new(&compile_engine, &bytes)?))
            }),
        )
        .await
        .context("Component compilation exceeded 60 seconds")??;
        let component_type = component.component_type();
        let exports: Vec<_> = component_type
            .exports(&engine)
            .map(|(name, _)| name.to_string())
            .collect();
        let allowed: &[&str] = match manifest.world.as_deref() {
            Some("command-plugin") => &["solidcraft:plugin/command@0.1.0"],
            Some("panel-plugin") => &[
                "solidcraft:plugin/command@0.1.0",
                "solidcraft:plugin/panel@0.1.0",
            ],
            Some("importer-plugin") => &["solidcraft:plugin/importer@0.1.0"],
            _ => &[],
        };
        ensure!(
            exports
                .iter()
                .filter(|n| n.starts_with("solidcraft:plugin/"))
                .all(|n| allowed.contains(&n.as_str())),
            "Component exports a different plugin world"
        );
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let ticker = engine.clone();
        std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || {
                while !flag.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(10));
                    ticker.increment_epoch();
                }
            })?;
        let plugin = Self {
            settings: Settings::default(),
            version_dir: storage::data_dir()?.join("versions"),
            manifest,
            engine,
            component,
            stop,
            physical_pending: AtomicBool::new(false),
        };
        // Typecheck without instantiation, so check never executes guest code.
        let linker = plugin.linker()?;
        match plugin.manifest.world.as_deref() {
            Some("command-plugin") => {
                cb::CommandPluginPre::new(linker.instantiate_pre(&plugin.component)?)?;
            }
            Some("panel-plugin") => {
                b::PanelPluginPre::new(linker.instantiate_pre(&plugin.component)?)?;
            }
            Some("importer-plugin") => {
                ib::ImporterPluginPre::new(linker.instantiate_pre(&plugin.component)?)?;
            }
            _ => anyhow::bail!("Unknown world"),
        }
        Ok(plugin)
    }
    fn linker(&self) -> Result<Linker<State>> {
        let mut l = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut l)?;
        match self.manifest.world.as_deref() {
            Some("command-plugin") => cb::CommandPlugin::add_to_linker::<
                _,
                wasmtime::component::HasSelf<_>,
            >(&mut l, |s| s)?,
            Some("panel-plugin") => {
                b::PanelPlugin::add_to_linker::<_, wasmtime::component::HasSelf<_>>(&mut l, |s| s)?
            }
            Some("importer-plugin") => {
                let mut options = ib::LinkOptions::default();
                options.file_formats(true);
                ib::ImporterPlugin::add_to_linker::<_, wasmtime::component::HasSelf<_>>(
                    &mut l,
                    &options,
                    |s| s,
                )?;
            }
            _ => anyhow::bail!("Unknown world"),
        }
        Ok(l)
    }
    fn store(&self, mut state: State) -> Store<State> {
        state.settings = self.settings.clone();
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.budget);
        store.set_hostcall_fuel(8 * MIB);
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);
        store
    }
    fn record_version(&self) -> Result<()> {
        let folder = &self.version_dir;
        std::fs::create_dir_all(folder)?;
        std::fs::write(folder.join(&self.manifest.id), &self.manifest.version)?;
        Ok(())
    }
    pub async fn run(&self, entry: &str, params: &str, port: Option<u16>) -> Result<String> {
        policy::json(params)?;
        let e = self
            .manifest
            .entries
            .iter()
            .find(|e| e.name == entry)
            .context("Unknown entry")?;
        let mut state = State::new(self.manifest.clone(), port)?;
        state.transaction = e.kind == "transaction";
        state.read_only = e.effect == "read";
        if e.effect == "physical" {
            ensure!(
                self.physical_pending
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok(),
                "denied: a confirmation is already open"
            );
            struct Pending<'a>(&'a AtomicBool);
            impl Drop for Pending<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _pending = Pending(&self.physical_pending);
            let approval_path = storage::data_dir()?
                .join("approvals")
                .join(format!("{}.json", self.manifest.id));
            let approval_before = std::fs::read(&approval_path).ok();
            let selection_stamp = || -> Result<_> {
                self.settings
                    .selection
                    .as_ref()
                    .map(|p| {
                        let m = std::fs::metadata(p)?;
                        Ok((m.len(), m.modified()?))
                    })
                    .transpose()
            };
            let selection_before = selection_stamp()?;
            let c = e
                .confirm
                .as_ref()
                .context("Missing physical confirmation")?;
            eprintln!(
                "{} — {} — {} — {}\nParams: {}",
                self.manifest.name,
                e.machine_kind.as_deref().unwrap_or(""),
                c.machine,
                c.action,
                params
            );
            if self.settings.confirm_physical {
                ensure!(
                    std::env::var("DUTCHMAN_PLUGINS_TEST").as_deref() == Ok("1"),
                    "--confirm-physical is for tests only"
                );
                ensure!(
                    !self.settings.addresses.is_empty()
                        && self
                            .settings
                            .addresses
                            .values()
                            .all(|a| crate::local::origin(a)
                                .ok()
                                .is_some_and(|u| u.host_str() == Some("127.0.0.1"))),
                    "Test confirmation requires loopback test services"
                );
            } else {
                use std::io::IsTerminal;
                ensure!(
                    std::io::stdin().is_terminal(),
                    "denied: this action needs confirmation in the terminal"
                );
                let mut tty = std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open("/dev/tty")
                    .context("denied: confirmation terminal unavailable")?;
                write!(tty, "Press {}? Type y to confirm: ", c.action)?;
                tty.flush()?;
                let mut answer = String::new();
                std::io::BufReader::new(tty).read_line(&mut answer)?;
                ensure!(answer.trim() == "y", "denied: physical action cancelled");
            }
            ensure!(
                approval_before == std::fs::read(&approval_path).ok(),
                "denied: approval changed during confirmation"
            );
            ensure!(
                selection_before == selection_stamp()?,
                "denied: selection changed during confirmation"
            );
            state.confirmation = Some(Confirmation::new()?);
        }
        let seconds = if state.transaction {
            self.manifest.limits.transaction_seconds
        } else {
            30
        };
        state.deadline = std::time::Instant::now() + Duration::from_secs(seconds);
        let mut store = self.store(state);
        let l = self.linker()?;
        tokio::time::timeout(Duration::from_secs(seconds), async {
            let result = match self.manifest.world.as_deref() {
                Some("command-plugin") => {
                    let p = cb::CommandPlugin::instantiate_async(&mut store, &self.component, &l)
                        .await?;
                    p.solidcraft_plugin_command()
                        .call_execute(&mut store, entry, &params.to_string())
                        .await?
                }
                Some("panel-plugin") => {
                    let p =
                        b::PanelPlugin::instantiate_async(&mut store, &self.component, &l).await?;
                    p.solidcraft_plugin_command()
                        .call_execute(&mut store, entry, &params.to_string())
                        .await?
                }
                _ => anyhow::bail!("Plugin has no command world"),
            }
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            policy::json(&result)?;
            self.record_version()?;
            Ok(result)
        })
        .await
        .context("limit: call deadline")?
    }
    pub async fn import(
        &self,
        path: &std::path::Path,
        port: Option<u16>,
    ) -> Result<(Vec<u8>, String)> {
        ensure!(
            self.manifest.world.as_deref() == Some("importer-plugin"),
            "Not an importer"
        );
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .context("Input has no extension")?
            .to_ascii_lowercase();
        ensure!(
            self.manifest
                .formats
                .as_ref()
                .is_some_and(|fs| fs.iter().any(|f| f.extension == ext)),
            "Importer does not support this extension"
        );
        let file = std::fs::File::open(path)?;
        ensure!(
            file.metadata()?.is_file() && file.metadata()?.len() <= 1 << 30,
            "Input must be a regular file up to 1 GiB"
        );
        let info = ib::solidcraft::plugin::input::FileInfo {
            name: path
                .file_name()
                .context("Missing input filename")?
                .to_string_lossy()
                .into_owned(),
            size: file.metadata()?.len(),
        };
        let mut state = State::new(self.manifest.clone(), port)?;
        state.input = Some(file);
        state.deadline = std::time::Instant::now() + Duration::from_secs(120);
        let mut store = self.store(state);
        let l = self.linker()?;
        tokio::time::timeout(Duration::from_secs(120), async {
            let p = ib::ImporterPlugin::instantiate_async(&mut store, &self.component, &l).await?;
            let report = p
                .solidcraft_plugin_importer()
                .call_import_design(&mut store, &info)
                .await?
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            ensure!(report.len() <= 65536, "limit: report exceeds 64 KiB");
            self.record_version()?;
            Ok((std::mem::take(&mut store.data_mut().output), report))
        })
        .await
        .context("limit: importer deadline")?
    }
}

fn panel_valid(items: &mut [panel_types::Item], markers: &[panel_types::Marker]) -> Result<()> {
    use panel_types::{Cell, Item};
    ensure!(
        items.len() <= 2000 && markers.len() <= 500,
        "limit: panel items or markers"
    );
    fn text(s: &str) -> Result<()> {
        ensure!(s.len() <= 4096, "limit: panel text");
        Ok(())
    }
    fn id(s: &str) -> Result<()> {
        ensure!(!s.is_empty() && s.len() <= 64, "invalid: panel id");
        Ok(())
    }
    fn button(b: &panel_types::Button) -> Result<()> {
        id(&b.id)?;
        text(&b.label)
    }
    fn checkbox(b: &panel_types::Checkbox) -> Result<()> {
        id(&b.id)?;
        text(&b.label)
    }
    let mut status_seen = false;
    for item in items {
        match item {
            Item::Heading(t) => text(t)?,
            Item::Text(t) => text(&t.text)?,
            Item::Button(b) => button(b)?,
            Item::Checkbox(b) => checkbox(b)?,
            Item::Row(cells) => {
                ensure!(cells.len() <= 32, "limit: row cells");
                for c in cells {
                    match c {
                        Cell::Text(t) => text(&t.text)?,
                        Cell::Button(b) => button(b)?,
                        Cell::Checkbox(b) => checkbox(b)?,
                    }
                }
            }
            Item::TextInput(t) => {
                id(&t.id)?;
                text(&t.label)?;
                text(&t.value)?;
                ensure!(
                    t.value.chars().count() <= t.max_chars as usize,
                    "limit: text input"
                );
            }
            Item::NumberInput(n) => {
                id(&n.id)?;
                text(&n.label)?;
                text(&n.unit)?;
                ensure!(
                    [n.value, n.min, n.max, n.step]
                        .iter()
                        .all(|n| n.is_finite())
                        && n.min <= n.max
                        && n.step > 0.0,
                    "invalid: numeric input"
                );
                n.value = n.value.clamp(n.min, n.max);
            }
            Item::Choice(c) => {
                id(&c.id)?;
                text(&c.label)?;
                ensure!(
                    c.options.len() <= 64 && (c.selected as usize) < c.options.len(),
                    "invalid: choice"
                );
                for s in &c.options {
                    text(s)?;
                }
            }
            Item::Status(s) => {
                ensure!(!status_seen, "invalid: duplicate status");
                status_seen = true;
                ensure!(
                    s.lines.len() <= 2
                        && s.lines.iter().all(|s| !s.is_empty()
                            && s.chars().count() <= 40
                            && !s.chars().any(char::is_control)),
                    "invalid: status lines"
                );
                ensure!(
                    s.progress
                        .is_none_or(|p| p.is_finite() && (0.0..=1.0).contains(&p)),
                    "invalid: status progress"
                );
            }
            Item::Separator => {}
        }
    }
    for m in markers {
        id(&m.id)?;
        ensure!(
            m.label.chars().count() <= 80
                && [m.at.x, m.at.y, m.at.z]
                    .iter()
                    .all(|n| n.is_finite() && n.abs() <= 1e6),
            "invalid: marker"
        );
    }
    Ok(())
}
impl Plugin {
    fn draw_panel(&self, items: &[panel_types::Item], view: (bool, bool)) {
        if view.0 {
            println!("{items:#?}");
        }
        if view.1
            && self
                .manifest
                .surfaces
                .iter()
                .any(|s| s.kind == "toolbar-widget")
        {
            let status = items.iter().find_map(|item| {
                if let panel_types::Item::Status(s) = item {
                    Some(s)
                } else {
                    None
                }
            });
            println!("Widget: {status:?}; menu: Open panel (open-panel)");
        }
    }
    pub async fn panel(&self, port: Option<u16>) -> Result<()> {
        ensure!(
            self.manifest.world.as_deref() == Some("panel-plugin"),
            "Not a panel"
        );
        let mut previous = Vec::new();
        let mut view = (
            true,
            self.manifest
                .surfaces
                .iter()
                .any(|s| s.kind == "toolbar-widget"),
        );
        let mut refresh = Refresh::default();
        refresh.visible(true, std::time::Instant::now());
        self.panel_event(
            panel_types::Event::Opened,
            port,
            &mut previous,
            &mut refresh,
            view,
        )
        .await?;
        println!(
            "Events: {{\"id\":\"button-id\",\"inputs\":{{}}}}; widget/open-panel; hide/show; quit"
        );
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        loop {
            if refresh.take(std::time::Instant::now()) {
                if let Err(e) = self
                    .panel_event(
                        panel_types::Event::Refresh,
                        port,
                        &mut previous,
                        &mut refresh,
                        view,
                    )
                    .await
                {
                    eprintln!("Panel call failed: {e:#}");
                }
                continue;
            }
            let mut fds = [rustix::event::PollFd::new(
                &stdin,
                rustix::event::PollFlags::IN,
            )];
            if rustix::event::poll(
                &mut fds,
                Some(&rustix::event::Timespec {
                    tv_sec: 0,
                    tv_nsec: 100_000_000,
                }),
            )? == 0
            {
                continue;
            }
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                break;
            }
            match line.trim() {
                "quit" => break,
                "hide" => {
                    view = (false, false);
                    refresh.visible(false, std::time::Instant::now());
                    continue;
                }
                "show" | "open-panel" => {
                    view.0 = true;
                    refresh.visible(true, std::time::Instant::now());
                    self.draw_panel(&previous, view);
                    continue;
                }
                "widget" => {
                    view = (
                        false,
                        self.manifest
                            .surfaces
                            .iter()
                            .any(|s| s.kind == "toolbar-widget"),
                    );
                    refresh.visible(view.1, std::time::Instant::now());
                    self.draw_panel(&previous, view);
                    continue;
                }
                _ => {}
            }
            if !view.0 {
                continue;
            }
            let result = async {
                let v = policy::json(&line)?;
                let id = v
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .context("Event requires id")?
                    .to_string();
                ensure!(id.len() <= 64, "Event id exceeds 64 bytes");
                let mut inputs = vec![];
                if let Some(obj) = v.get("inputs").and_then(serde_json::Value::as_object) {
                    ensure!(obj.len() <= 2000, "Too many event inputs");
                    for (key, value) in obj {
                        let choice = previous.iter().any(
                            |item| matches!(item, panel_types::Item::Choice(c) if c.id == *key),
                        );
                        let value = if choice {
                            panel_types::Value::Selected(u32::try_from(
                                value.as_u64().context("Choice needs an index")?,
                            )?)
                        } else if let Some(n) = value.as_f64() {
                            panel_types::Value::Number(n)
                        } else if let Some(b) = value.as_bool() {
                            panel_types::Value::Checked(b)
                        } else if let Some(s) = value.as_str() {
                            panel_types::Value::Text(s.into())
                        } else {
                            anyhow::bail!("Unsupported input value")
                        };
                        inputs.push((key.clone(), value));
                    }
                }
                self.panel_event(
                    panel_types::Event::Action(panel_types::Action {
                        id,
                        inputs,
                        camera: None,
                    }),
                    port,
                    &mut previous,
                    &mut refresh,
                    view,
                )
                .await
            }
            .await;
            if let Err(e) = result {
                eprintln!("Panel call failed: {e:#}. Previous rows: {previous:?}");
            }
        }
        Ok(())
    }
    async fn panel_event(
        &self,
        event: panel_types::Event,
        port: Option<u16>,
        previous: &mut Vec<panel_types::Item>,
        refresh: &mut Refresh,
        view: (bool, bool),
    ) -> Result<()> {
        let mut store = self.store(State::new(self.manifest.clone(), port)?);
        store.data_mut().panel_call = true;
        let l = self.linker()?;
        let (items, queued) = tokio::time::timeout(Duration::from_secs(30), async {
            let p = b::PanelPlugin::instantiate_async(&mut store, &self.component, &l).await?;
            p.solidcraft_plugin_panel()
                .call_handle(&mut store, &event)
                .await?
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            // Render and markers each have their own per-call counters, sharing only this successful instance.
            let queued = std::mem::take(&mut store.data_mut().queued);
            store.data_mut().reset(false);
            let mut items = p.solidcraft_plugin_panel().call_render(&mut store).await?;
            store.data_mut().reset(false);
            let markers = p.solidcraft_plugin_panel().call_markers(&mut store).await?;
            panel_valid(&mut items, &markers)?;
            Ok::<_, anyhow::Error>((items, queued))
        })
        .await
        .context("limit: panel deadline")??;
        if let Some(seconds) = store.data_mut().refresh.take() {
            refresh.schedule(seconds, std::time::Instant::now());
        }
        self.record_version()?;
        self.draw_panel(&items, view);
        *previous = items;
        for (entry, params) in queued {
            let result = self.run(&entry, &params, port).await;
            let outcome = result.map_err(|e| format!("{e:#}"));
            // Deliver executed once. No recursive request chains are accepted from this notification.
            let mut store = self.store(State::new(self.manifest.clone(), port)?);
            store.data_mut().panel_call = true;
            let l = self.linker()?;
            tokio::time::timeout(Duration::from_secs(30), async {
                let p = b::PanelPlugin::instantiate_async(&mut store, &self.component, &l).await?;
                let event = panel_types::Event::Executed(panel_types::Executed { entry, outcome });
                p.solidcraft_plugin_panel()
                    .call_handle(&mut store, &event)
                    .await?
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                ensure!(
                    store.data().queued.is_empty(),
                    "Executed notifications cannot queue more work in this text host"
                );
                store.data_mut().reset(false);
                let mut items = p.solidcraft_plugin_panel().call_render(&mut store).await?;
                let markers = p.solidcraft_plugin_panel().call_markers(&mut store).await?;
                panel_valid(&mut items, &markers)?;
                self.draw_panel(&items, view);
                *previous = items;
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("limit: executed deadline")??;
            if let Some(seconds) = store.data_mut().refresh.take() {
                refresh.schedule(seconds, std::time::Instant::now());
            }
            self.record_version()?;
        }
        Ok(())
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let bytes = self.wasi_output.contents();
        let text = String::from_utf8_lossy(&bytes);
        for line in text.lines().take(200usize.saturating_sub(self.logs)) {
            let line: String = line
                .chars()
                .scan(0, |n, c| {
                    *n += c.len_utf8();
                    (*n <= 1024).then_some(c)
                })
                .collect();
            eprintln!("[plugin {}] {line}", self.manifest.id);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    fn manifest() -> Manifest {
        serde_json::from_str(include_str!("../../examples/importer-fusion/plugin.json")).unwrap()
    }
    #[tokio::test]
    async fn importer_stream_caps_and_no_folder_access() {
        use ib::solidcraft::plugin::{input::Host as _, output::Host as _};
        let mut state = State::new(manifest(), None).unwrap();
        let mut f = tempfile::tempfile().unwrap();
        f.write_all(b"synthetic").unwrap();
        state.input = Some(f);
        assert!(matches!(
            state.read("../secret".into(), 0, 1).await,
            Err(E::Denied(_))
        ));
        assert!(matches!(
            state.read("".into(), 0, (8 * MIB + 1) as u32).await,
            Err(E::Limit(_))
        ));
        assert_eq!(state.read("".into(), 0, 4).await.unwrap(), b"synt");
        state.read_bytes = 1 << 30;
        assert!(matches!(
            state.read("".into(), 0, 1).await,
            Err(E::Limit(_))
        ));
        assert!(matches!(
            state.write(vec![0; 8 * MIB + 1]).await,
            Err(E::Limit(_))
        ));
        state.output.resize(256 * MIB, 0);
        assert!(matches!(state.write(vec![1]).await, Err(E::Limit(_))));
    }
    #[tokio::test]
    async fn a_blocked_control_worker_does_not_hold_the_deadline() {
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            Duration::from_millis(30),
            detached_work(|| {
                std::thread::sleep(Duration::from_secs(2));
                Ok(())
            }),
        )
        .await;
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn memory_budget_is_aggregate() {
        let mut budget = Budget {
            used: 0,
            max: 16 * MIB,
            tables: 0,
        };
        assert!(budget.memory_growing(0, 10 * MIB, None).unwrap());
        assert!(budget.memory_growing(0, 7 * MIB, None).is_err());
        assert!(budget.table_growing(0, 10001, None).is_err());
    }
    #[test]
    fn invalid_panel_is_refused_and_values_clamped() {
        let mut items = vec![panel_types::Item::NumberInput(panel_types::NumberInput {
            id: "size".into(),
            label: "Size".into(),
            value: 100.0,
            min: 1.0,
            max: 10.0,
            step: 1.0,
            unit: "mm".into(),
        })];
        panel_valid(&mut items, &[]).unwrap();
        match &items[0] {
            panel_types::Item::NumberInput(n) => assert_eq!(n.value, 10.0),
            _ => panic!("Wrong variant"),
        }
        if let panel_types::Item::NumberInput(n) = &mut items[0] {
            n.step = f64::NAN;
        }
        assert!(panel_valid(&mut items, &[]).is_err());
    }
    #[test]
    fn t32_widget_validation() {
        use panel_types::{Item, Status, WidgetState};
        let good = Item::Status(Status {
            lines: vec!["Printer".into(), "Printing".into()],
            progress: Some(0.5),
            state: WidgetState::Busy,
        });
        assert!(panel_valid(&mut [good.clone()], &[]).is_ok());
        assert!(panel_valid(&mut [good.clone(), good.clone()], &[]).is_err());
        for lines in [
            vec!["x".repeat(41)],
            vec!["a".into(), "b".into(), "c".into()],
            vec!["bad\nlabel".into()],
        ] {
            assert!(
                panel_valid(
                    &mut [Item::Status(Status {
                        lines,
                        progress: None,
                        state: WidgetState::Idle
                    })],
                    &[]
                )
                .is_err()
            );
        }
        for progress in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert!(
                panel_valid(
                    &mut [Item::Status(Status {
                        lines: vec![],
                        progress: Some(progress),
                        state: WidgetState::Idle
                    })],
                    &[]
                )
                .is_err()
            );
        }
    }
    #[tokio::test]
    async fn t28_t31_t34_t37_host_denials() {
        use host::Host as _;
        use net::Host as _;
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../examples/print/plugin.json")).unwrap();
        let mut state = State::new(manifest, None).unwrap();
        let req = || net::Request {
            method: net::Method::Get,
            url: "local:dutchman-print/api/v1/printers".into(),
            headers: vec![],
            body: vec![],
            mesh: None,
        };
        assert!(
            matches!(state.fetch(req()).await, Err(E::Failed(s)) if s.contains("dutchman-print"))
        );
        let mut forged = req();
        forged
            .headers
            .push(("pLuGiN-CoNfIrMaTiOn".into(), "forged".into()));
        assert!(matches!(state.fetch(forged).await, Err(E::Denied(_))));
        let mut raw = req();
        raw.url = "http://127.0.0.1/api/v1/printers".into();
        assert!(matches!(state.fetch(raw).await, Err(E::Denied(_))));
        assert!(matches!(state.schedule_refresh(0).await, Err(E::Denied(_))));
        state.panel_call = true;
        state.schedule_refresh(0).await.unwrap();
        assert_eq!(state.refresh, Some(2));
        state.read_only = true;
        assert!(matches!(
            state.request_execute("print".into(), "{}".into()).await,
            Err(E::Denied(_))
        ));
        state.read_only = false;
        state
            .settings
            .addresses
            .insert("dutchman-print".into(), "http://127.0.0.1:1".into());
        let mesh = || net::MeshBody {
            bodies: vec![],
            format: net::MeshFormat::ThreeMf,
            quality: net::MeshQuality::Standard,
        };
        let mut both = req();
        both.mesh = Some(mesh());
        both.body.push(1);
        assert!(matches!(state.fetch(both).await, Err(E::Invalid(_))));
        state.manifest.permissions.local[0].mesh = false;
        let mut no_permission = req();
        no_permission.mesh = Some(mesh());
        assert!(matches!(
            state.fetch(no_permission).await,
            Err(E::Denied(_))
        ));
        state.manifest.permissions.local[0].mesh = true;
        let mut first = req();
        first.mesh = Some(mesh());
        assert!(matches!(state.fetch(first).await, Err(E::Failed(_))));
        let mut second = req();
        second.mesh = Some(mesh());
        assert!(matches!(state.fetch(second).await, Err(E::Limit(_))));
    }
    #[tokio::test]
    async fn trap_timeout_and_memory_breach_get_fresh_instances() {
        let dir = tempfile::tempdir().unwrap();
        let mut m: serde_json::Value =
            serde_json::from_str(include_str!("../../examples/pilot/plugin.json")).unwrap();
        m["component"] = serde_json::json!("test.wasm");
        m["limits"] = serde_json::json!({"memory_mib":16,"transaction_seconds":1});
        m["entries"] = serde_json::json!(
            ["ok", "trap", "loop", "grow"]
                .into_iter()
                .map(|name| serde_json::json!({"name":name,"label":name}))
                .collect::<Vec<_>>()
        );
        std::fs::write(dir.path().join("plugin.json"), m.to_string()).unwrap();
        let wat = r#"(component
            (type $error (variant (case "failed" string) (case "denied" string) (case "limit" string) (case "invalid" string)))
            (core module $m
                (memory (export "memory") 1)
                (global $heap (mut i32) (i32.const 1024))
                (global $count (mut i32) (i32.const 0))
                (data (i32.const 4) "\40\00\00\00\01\00\00\00")
                (data (i32.const 64) "1")
                (func (export "realloc") (param i32 i32 i32 i32) (result i32)
                    global.get $heap
                    global.get $heap local.get 3 i32.add global.set $heap)
                (func (export "execute") (param i32 i32 i32 i32) (result i32)
                    global.get $count if unreachable end
                    i32.const 1 global.set $count
                    local.get 0 i32.load8_u i32.const 116 i32.eq if unreachable end
                    local.get 0 i32.load8_u i32.const 108 i32.eq if (loop $forever br $forever) end
                    local.get 0 i32.load8_u i32.const 103 i32.eq if i32.const 257 memory.grow drop end
                    i32.const 0))
            (core instance $i (instantiate $m))
            (alias core export $i "memory" (core memory $memory))
            (alias core export $i "execute" (core func $execute-core))
            (alias core export $i "realloc" (core func $realloc))
            (func $execute (param "entry" string) (param "params" string) (result (result string (error $error)))
                (canon lift (core func $execute-core) (memory $memory) (realloc $realloc)))
            (instance (export "solidcraft:plugin/command@0.1.0") (export "plugin-error" (type $error)) (export "execute" (func $execute)))
        )"#;
        std::fs::write(dir.path().join("test.wasm"), wat::parse_str(wat).unwrap()).unwrap();
        let mut plugin = Plugin::load(dir.path(), true, true).await.unwrap();
        plugin.version_dir = dir.path().join("versions");
        assert_eq!(plugin.run("ok", "{}", None).await.unwrap(), "1");
        for entry in ["trap", "loop", "grow"] {
            assert!(plugin.run(entry, "{}", None).await.is_err(), "{entry}");
            assert_eq!(
                plugin.run("ok", "{}", None).await.unwrap(),
                "1",
                "fresh after {entry}"
            );
        }
    }
}
