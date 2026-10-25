#![forbid(unsafe_code)]
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dutchman_plugins::{manifest::Manifest, runtime::Plugin};
use std::{io::Write, path::PathBuf};
#[derive(Parser)]
#[command(about = "A standalone SolidCraft component plugin reference host")]
struct Cli {
    #[arg(long, global = true)]
    yes: bool,
    #[arg(long, global = true, env = "SOLIDCRAFT_CONTROL_PORT")]
    port: Option<u16>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    List,
    Check {
        dir: PathBuf,
    },
    Run {
        plugin: PathBuf,
        entry: Option<String>,
        json: Option<String>,
    },
    Import {
        plugin: PathBuf,
        file: PathBuf,
        #[arg(long)]
        open: bool,
    },
    Panel {
        plugin: PathBuf,
    },
}
fn folder(p: PathBuf) -> PathBuf {
    if p.is_dir() {
        p
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples")
            .join(p)
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::List => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
            let mut extensions = std::collections::BTreeMap::new();
            for item in std::fs::read_dir(root)? {
                let dir = item?.path();
                if !dir.is_dir() {
                    continue;
                }
                match Manifest::read(&dir) {
                    Ok(m) => {
                        if let Some(fs) = m.formats {
                            for f in fs {
                                if let Some(other) =
                                    extensions.insert(f.extension.clone(), m.id.clone())
                                {
                                    anyhow::bail!(
                                        "Extension {} claimed by {} and {}",
                                        f.extension,
                                        other,
                                        m.id
                                    );
                                }
                            }
                        }
                        println!("{}\t{}\t{}", m.id, m.version, m.description);
                    }
                    Err(e) => eprintln!("skipped {}: {e}", dir.display()),
                }
            }
        }
        Command::Check { dir } => {
            Plugin::load(&dir, false, true).await?;
            println!("Valid component, world, manifest and limits.");
        }
        Command::Run {
            plugin,
            entry,
            json,
        } => {
            let plugin = Plugin::load(&folder(plugin), cli.yes, false).await?;
            if plugin.manifest.world.as_deref() == Some("panel-plugin") && entry.is_none() {
                plugin.panel(cli.port).await?;
            } else {
                let entry = entry.context("Entry required")?;
                let supplied = json.unwrap_or_else(|| "{}".into());
                let params = if serde_json::from_str::<serde_json::Value>(&supplied).is_ok() {
                    supplied
                } else {
                    serde_json::to_string(&supplied)?
                };
                println!("{}", plugin.run(&entry, &params, cli.port).await?);
            }
        }
        Command::Import { plugin, file, open } => {
            let plugin = Plugin::load(&folder(plugin), cli.yes, false).await?;
            let (bytes, report) = plugin.import(&file, cli.port).await?;
            let out = file.with_extension("solidcraft");
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&out)
                .context("Output exists or cannot be created; move it before retrying")?;
            f.write_all(&bytes)?;
            println!("{}\n{report}", out.display());
            if open {
                let path = std::fs::canonicalize(out)?;
                tokio::task::spawn_blocking(move || -> Result<()> {
                    let s = solidcraft_bridge::Session::connect(
                        cli.port,
                        std::env::var("SOLIDCRAFT_CONTROL_TOKEN").ok(),
                    )?;
                    s.execute("doc.open", serde_json::json!({"path":path}))?;
                    Ok(())
                })
                .await??;
            }
        }
        Command::Panel { plugin } => {
            Plugin::load(&folder(plugin), cli.yes, false)
                .await?
                .panel(cli.port)
                .await?;
        }
    }
    Ok(())
}
