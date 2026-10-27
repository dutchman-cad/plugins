use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, io::Read, path::Path};
pub const MIB: usize = 1024 * 1024;
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub surfaces: Vec<Surface>,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub api: Option<String>,
    pub component: Option<String>,
    pub world: Option<String>,
    #[serde(default)]
    pub entries: Vec<Entry>,
    pub panel: Option<Panel>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub limits: Limits,
    pub formats: Option<Vec<Format>>,
    pub reads_input_folder: Option<bool>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    #[serde(default = "edit")]
    pub effect: String,
    pub machine_kind: Option<String>,
    pub confirm: Option<Confirmation>,
    pub name: String,
    pub label: String,
    #[serde(default = "transaction")]
    pub kind: String,
    pub params: Option<String>,
}
fn edit() -> String {
    "edit".into()
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    pub machine: String,
    pub action: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub kind: String,
    pub label: Option<String>,
    pub entry: Option<String>,
}
fn transaction() -> String {
    "transaction".into()
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub label: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Format {
    pub extension: String,
    pub label: String,
}
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub local: Vec<LocalService>,
    #[serde(default)]
    pub store: bool,
    #[serde(default)]
    pub clock: bool,
    #[serde(default)]
    pub net: Vec<serde_json::Value>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalService {
    pub name: String,
    pub port: u16,
    #[serde(default)]
    pub mesh: bool,
    pub secret: Option<Secret>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Secret {
    pub name: String,
    pub label: String,
    pub header: String,
    #[serde(default)]
    pub prefix: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub memory_mib: usize,
    pub stack_kib: usize,
    pub component_mib: usize,
    pub transaction_seconds: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mib: 64,
            stack_kib: 512,
            component_mib: 16,
            transaction_seconds: 10,
        }
    }
}
pub fn slug(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
fn text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}
impl Manifest {
    pub fn read(dir: &Path) -> Result<Self> {
        let path = dir.join("plugin.json");
        ensure!(
            std::fs::metadata(&path)?.len() <= 16384,
            "Manifest exceeds 16 KiB"
        );
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(16385)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 16384, "Manifest exceeds 16 KiB");
        let m: Self = serde_json::from_slice(&bytes).context("Invalid plugin.json")?;
        m.validate()?;
        Ok(m)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(slug(&self.id, 64), "Invalid plugin id");
        ensure!(
            text(&self.name, 80)
                && text(&self.version, 32)
                && self.description.chars().count() <= 1024,
            "Invalid listing fields"
        );
        let Some(component) = &self.component else {
            return Ok(());
        };
        ensure!(
            !component.is_empty()
                && component != "."
                && component != ".."
                && !component.contains(['/', '\\', ':'])
                && !component.starts_with('.'),
            "component must be a filename"
        );
        ensure!(self.api.as_deref() == Some("0.1"), "Unsupported API");
        let world = self.world.as_deref().context("Missing world")?;
        ensure!(
            ["command-plugin", "panel-plugin", "importer-plugin"].contains(&world),
            "Unknown world"
        );
        ensure!(
            (world == "panel-plugin") == self.panel.is_some(),
            "panel is required only for panel-plugin"
        );
        if let Some(p) = &self.panel {
            ensure!(text(&p.label, 40), "Invalid panel label");
        }
        ensure!(
            self.entries.len() <= 64 && self.commands.len() <= 256,
            "Too many entries or commands"
        );
        let mut names = BTreeSet::new();
        for e in &self.entries {
            ensure!(
                slug(&e.name, 32) && text(&e.label, 80) && names.insert(&e.name),
                "Invalid or duplicate entry"
            );
            ensure!(
                e.kind == "transaction" || e.kind == "action",
                "Invalid entry kind"
            );
            ensure!(
                ["read", "edit", "physical"].contains(&e.effect.as_str()),
                "Invalid entry effect"
            );
            ensure!(
                e.kind == "action" || e.effect == "edit",
                "Only actions may be read or physical"
            );
            if e.effect == "physical" {
                ensure!(
                    e.machine_kind.as_ref().is_some_and(|s| text(s, 40)),
                    "Physical action needs machine_kind"
                );
                let c = e
                    .confirm
                    .as_ref()
                    .context("Physical action needs confirm")?;
                ensure!(
                    text(&c.machine, 80) && text(&c.action, 80),
                    "Invalid confirmation"
                );
            } else {
                ensure!(
                    e.machine_kind.is_none() && e.confirm.is_none(),
                    "Confirmation only on physical actions"
                );
            }
            if let Some(p) = &e.params {
                ensure!(
                    p.len() <= 1024 && !p.chars().any(char::is_control),
                    "Invalid params documentation"
                );
            }
        }
        ensure!(self.surfaces.len() <= 8, "Too many surfaces");
        let mut kinds = BTreeSet::new();
        for surface in &self.surfaces {
            if let Some(label) = &surface.label {
                ensure!(text(label, 40), "Invalid surface label");
            }
            match surface.kind.as_str() {
                "toolbar-entry" => {
                    ensure!(world != "importer-plugin", "Importer toolbar entry");
                    let entry = self
                        .entries
                        .iter()
                        .find(|e| Some(&e.name) == surface.entry.as_ref())
                        .context("Unknown surface entry")?;
                    ensure!(entry.params.is_none(), "Toolbar entry requires parameters");
                }
                "panel" | "toolbar-widget" => {
                    ensure!(
                        world == "panel-plugin" && surface.entry.is_none(),
                        "Panel surface needs panel world"
                    );
                }
                "open-source" | "import-menu" => ensure!(
                    world == "importer-plugin" && surface.entry.is_none(),
                    "Importer surface needs importer world"
                ),
                _ => bail!("Unknown surface kind"),
            }
            ensure!(
                kinds.insert((&surface.kind, &surface.entry)),
                "Duplicate surface"
            );
        }
        ensure!(self.permissions.local.len() <= 2, "Too many local services");
        let mut services = BTreeSet::new();
        for service in &self.permissions.local {
            ensure!(
                slug(&service.name, 32)
                    && !service.name.contains('_')
                    && service.port != 0
                    && services.insert(&service.name),
                "Invalid local service"
            );
            if let Some(secret) = &service.secret {
                ensure!(
                    slug(&secret.name, 32)
                        && text(&secret.label, 40)
                        && ["Authorization", "X-Api-Key", "Private-Token"]
                            .contains(&secret.header.as_str())
                        && secret.prefix.len() <= 16
                        && !secret.prefix.chars().any(char::is_control),
                    "Invalid local secret"
                );
            }
        }
        ensure!(
            self.permissions.local.is_empty() || world == "panel-plugin",
            "Local services require panel world"
        );
        let l = &self.limits;
        ensure!(
            (16..=256).contains(&l.memory_mib)
                && (512..=16384).contains(&l.stack_kib)
                && (1..=16).contains(&l.component_mib)
                && (1..=120).contains(&l.transaction_seconds),
            "Limits outside spec ranges"
        );
        ensure!(
            self.permissions.net.is_empty(),
            "Network permission is not supported by this host"
        );
        if world == "importer-plugin" {
            ensure!(
                self.entries.is_empty() && self.commands.is_empty() && !self.permissions.store,
                "Importer world has no commands or store"
            );
            let formats = self.formats.as_ref().context("Importer needs formats")?;
            ensure!(
                !formats.is_empty() && formats.len() <= 16,
                "Expected 1–16 formats"
            );
            let mut extensions = BTreeSet::new();
            for f in formats {
                ensure!(
                    !f.extension.is_empty()
                        && f.extension.len() <= 16
                        && f.extension
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                        && text(&f.label, 80)
                        && extensions.insert(&f.extension),
                    "Invalid or duplicate format"
                );
            }
            // This version never grants folder access; reject instead of silently weakening it.
            ensure!(
                self.reads_input_folder != Some(true),
                "Input folder access is not supported in this version"
            );
        } else {
            ensure!(
                self.formats.is_none() && self.reads_input_folder.is_none(),
                "Importer fields on another world"
            );
        }
        let catalogue = sc_pilot::catalog::Catalog::bundled().map_err(anyhow::Error::msg)?;
        for c in &self.commands {
            if catalogue.get(c).is_none() {
                bail!("Unknown command in bundled stock catalogue: {c}");
            }
        }
        Ok(())
    }
}

/// Approval covers capabilities; user addresses and component bytes are separate.
pub fn approval(manifest: &Manifest) -> String {
    let capabilities = serde_json::json!({
        "commands": manifest.commands, "permissions": manifest.permissions,
        "limits": manifest.limits, "world": manifest.world,
        "formats": manifest.formats, "reads_input_folder": manifest.reads_input_folder,
        "entries": manifest.entries.iter().map(|e| serde_json::json!({
            "name":e.name,"kind":e.kind,"effect":e.effect,"machine_kind":e.machine_kind,"confirm":e.confirm
        })).collect::<Vec<_>>(),
        "surfaces": manifest.surfaces.iter().map(|s| serde_json::json!({"kind":s.kind,"entry":s.entry})).collect::<Vec<_>>()
    }).to_string();
    use sha2::{Digest, Sha256};
    Sha256::digest(capabilities.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
