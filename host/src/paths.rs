use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

/// Prefer the unpacked distribution, then a Cargo checkout, then the build source.
pub fn examples_roots(override_dir: Option<PathBuf>, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<_> = override_dir.into_iter().collect();
    if let Some(bin) = exe.and_then(Path::parent) {
        roots.push(bin.join("examples"));
        roots.push(bin.join("../../examples"));
    }
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples"));
    roots
}

pub fn examples_root(roots: &[PathBuf]) -> PathBuf {
    roots
        .iter()
        .find(|root| {
            std::fs::read_dir(root).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.path().join("plugin.json").is_file())
            })
        })
        .or_else(|| roots.first())
        .cloned()
        .unwrap_or_default()
}

pub fn plugin_folder(plugin: PathBuf, roots: &[PathBuf]) -> PathBuf {
    if plugin.is_dir() {
        return plugin;
    }
    roots
        .iter()
        .map(|root| root.join(&plugin))
        .find(|dir| dir.join("plugin.json").is_file())
        .unwrap_or_else(|| examples_root(roots).join(plugin))
}

fn executable_name(name: &str) -> OsString {
    format!("{name}{}", std::env::consts::EXE_SUFFIX).into()
}

fn candidates(name: &str, exe: Option<&Path>, search_path: Option<&OsStr>) -> Vec<PathBuf> {
    let name = executable_name(name);
    let mut paths = Vec::new();
    if let Some(bin) = exe.and_then(Path::parent) {
        paths.push(bin.join(&name));
        if name == executable_name("solidcraft") {
            paths.push(bin.join("SolidCraft.app/Contents/MacOS/solidcraft"));
        }
    }
    if let Some(search_path) = search_path {
        paths.extend(std::env::split_paths(search_path).map(|dir| dir.join(&name)));
    }
    if name == executable_name("solidcraft") && cfg!(windows) {
        if let Some(dir) = std::env::var_os("ProgramFiles") {
            paths.push(PathBuf::from(dir).join("SolidCraft").join(&name));
        }
        if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(dir).join("Programs/SolidCraft").join(&name));
        }
    }
    if name == executable_name("solidcraft") && cfg!(target_os = "macos") {
        paths.push(PathBuf::from(
            "/Applications/SolidCraft.app/Contents/MacOS/solidcraft",
        ));
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(
                PathBuf::from(home).join("Applications/SolidCraft.app/Contents/MacOS/solidcraft"),
            );
        }
    }
    paths
}

pub fn launch(name: &str, override_var: &str, args: Vec<OsString>) -> Result<()> {
    let program = if let Some(path) = std::env::var_os(override_var) {
        PathBuf::from(path)
    } else {
        candidates(
            name,
            std::env::current_exe().ok().as_deref(),
            std::env::var_os("PATH").as_deref(),
        )
        .into_iter()
        .find(|p| p.is_file())
        .with_context(|| format!("Cannot find {name}; set {override_var} to its executable path"))?
    };
    let status = Command::new(&program)
        .args(args)
        .status()
        .with_context(|| format!("Cannot start {}", program.display()))?;
    ensure!(status.success(), "{name} exited with {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacked_and_checkout_roots_follow_override() {
        let exe =
            PathBuf::from("distribution with spaces").join(executable_name("dutchman-plugins"));
        let roots = examples_roots(Some("custom plugins".into()), Some(&exe));
        assert_eq!(roots[0], PathBuf::from("custom plugins"));
        assert_eq!(
            roots[1],
            exe.parent().unwrap_or(Path::new("")).join("examples")
        );
        assert_eq!(
            roots[2],
            exe.parent().unwrap_or(Path::new("")).join("../../examples")
        );
        assert_eq!(examples_roots(None, None).len(), 1);
    }

    #[test]
    fn executable_suffix_and_sibling_precedence() -> Result<()> {
        let bin = PathBuf::from("package with spaces");
        let exe = bin.join(executable_name("dutchman-plugins"));
        let search_path =
            std::env::join_paths([Path::new("first path"), Path::new("second path")])?;
        for name in ["solidcraft", "dutchman-print"] {
            let paths = candidates(name, Some(&exe), Some(&search_path));
            assert_eq!(paths[0], bin.join(executable_name(name)));
            assert!(paths.contains(&Path::new("first path").join(executable_name(name))));
            assert!(paths.contains(&Path::new("second path").join(executable_name(name))));
        }
        assert_eq!(
            executable_name("dutchman-print"),
            if cfg!(windows) {
                "dutchman-print.exe"
            } else {
                "dutchman-print"
            }
        );
        Ok(())
    }
    #[test]
    fn relocated_plugins_work_without_source_or_current_directory() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let bin = temp.path().join("unpacked package");
        let plugin = bin.join("examples/print");
        std::fs::create_dir_all(&plugin)?;
        std::fs::write(plugin.join("plugin.json"), "{}")?;
        let roots = examples_roots(
            Some(temp.path().join("missing override")),
            Some(&bin.join(executable_name("dutchman-plugins"))),
        );
        assert_eq!(examples_root(&roots), bin.join("examples"));
        assert_eq!(plugin_folder("print".into(), &roots), plugin);
        assert_eq!(plugin_folder(plugin.clone(), &roots), plugin);
        // An empty Cargo examples folder must not hide the checkout's plugins.
        let empty = temp.path().join("target/release/examples");
        std::fs::create_dir_all(&empty)?;
        assert_eq!(
            examples_root(&[empty, bin.join("examples")]),
            bin.join("examples")
        );
        Ok(())
    }
}
