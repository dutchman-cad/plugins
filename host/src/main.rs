#![forbid(unsafe_code)]
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dutchman_plugins::{manifest::Manifest, runtime::Plugin};
use std::{io::Write, path::PathBuf};
mod paths;
#[derive(Parser)]
#[command(about = "A standalone SolidCraft component plugin reference host")]
struct Cli {
    #[arg(long, global = true)]
    yes: bool,
    #[arg(long, global = true, env = "SOLIDCRAFT_CONTROL_PORT")]
    port: Option<u16>,
    /// Host-owned service addresses, for example dutchman-print=http://127.0.0.1:8730.
    #[arg(long, global = true)]
    local: Vec<String>,
    #[arg(long, global = true)]
    local_secret: Vec<String>,
    #[arg(long, global = true)]
    selection_3mf: Option<PathBuf>,
    /// Test-only physical confirmation; requires DUTCHMAN_PLUGINS_TEST=1.
    #[arg(long, global = true)]
    confirm_physical: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Run the bundled Print companion; arguments follow `--`.
    Companion {
        #[arg(last = true)]
        args: Vec<std::ffi::OsString>,
    },
    /// Start an installed SolidCraft; arguments follow `--`.
    Solidcraft {
        #[arg(last = true)]
        args: Vec<std::ffi::OsString>,
    },
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
fn examples_roots() -> Vec<PathBuf> {
    paths::examples_roots(
        std::env::var_os("DUTCHMAN_PLUGINS_EXAMPLES").map(PathBuf::from),
        std::env::current_exe().ok().as_deref(),
    )
}
fn examples_root() -> PathBuf {
    paths::examples_root(&examples_roots())
}
fn folder(p: PathBuf) -> PathBuf {
    paths::plugin_folder(p, &examples_roots())
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut settings = dutchman_plugins::local::Settings {
        selection: cli.selection_3mf,
        confirm_physical: cli.confirm_physical,
        ..Default::default()
    };
    for supplied in cli.local {
        let (name, address) = supplied
            .split_once('=')
            .context("--local requires name=origin")?;
        dutchman_plugins::local::origin(address).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        anyhow::ensure!(
            settings
                .addresses
                .insert(name.into(), address.into())
                .is_none(),
            "Duplicate local address"
        );
    }
    for supplied in cli.local_secret {
        let (name, value) = supplied
            .split_once('=')
            .context("--local-secret requires name=value")?;
        settings.secrets.insert(name.into(), value.into());
    }
    match cli.command {
        Command::Companion { args } => paths::launch("dutchman-print", "DUTCHMAN_PRINT_BIN", args)?,
        Command::Solidcraft { args } => paths::launch("solidcraft", "SOLIDCRAFT_BIN", args)?,
        Command::List => {
            let root = examples_root();
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
            let mut plugin = Plugin::load(&folder(plugin), cli.yes, false).await?;
            plugin.settings = settings;
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
            let mut plugin = Plugin::load(&folder(plugin), cli.yes, false).await?;
            plugin.settings = settings;
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
            let mut plugin = Plugin::load(&folder(plugin), cli.yes, false).await?;
            plugin.settings = settings;
            plugin.panel(cli.port).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_commands_match_cli() -> Result<()> {
        for args in [
            vec!["dutchman-plugins", "list"],
            vec!["dutchman-plugins", "--yes", "panel", "print"],
            vec![
                "dutchman-plugins",
                "--yes",
                "--local",
                "dutchman-print=http://127.0.0.1:8730",
                "panel",
                "print",
            ],
            vec![
                "dutchman-plugins",
                "companion",
                "--",
                "--config",
                "config.toml",
                "serve",
                "--listen",
                "127.0.0.1:8730",
                "--data-dir",
                "private-jobs",
            ],
            vec!["dutchman-plugins", "solidcraft", "--", "--control", "7878"],
        ] {
            Cli::try_parse_from(args)?;
        }
        let cli = Cli::try_parse_from([
            "dutchman-plugins",
            "companion",
            "--",
            "--config",
            "a folder/config.toml",
        ])?;
        if let Command::Companion { args } = cli.command {
            assert_eq!(
                args,
                vec![
                    std::ffi::OsString::from("--config"),
                    std::ffi::OsString::from("a folder/config.toml")
                ]
            );
        } else {
            anyhow::bail!("Expected companion command");
        }
        Ok(())
    }
}
