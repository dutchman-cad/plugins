use crate::manifest::MIB;
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};
pub fn data_dir() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("DUTCHMAN_PLUGINS_DATA") {
        return Ok(p.into());
    }
    Ok(
        directories::ProjectDirs::from("org", "Dutchman", "dutchman-plugins")
            .ok_or_else(|| anyhow::anyhow!("Platform data directory unavailable"))?
            .data_dir()
            .to_path_buf(),
    )
}
pub fn key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 128
        && k.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && k.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}
pub fn validate(map: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    ensure!(map.len() <= 1024, "Store exceeds 1024 keys");
    ensure!(
        map.iter().all(|(k, v)| key(k) && v.len() <= MIB),
        "Invalid key or value exceeds 1 MiB"
    );
    ensure!(
        map.values().map(Vec::len).sum::<usize>() <= 4 * MIB,
        "Store exceeds 4 MiB"
    );
    Ok(())
}
pub fn read(path: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    ensure!(
        std::fs::metadata(path)?.len() <= 17 * MIB as u64,
        "Store file too large"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((17 * MIB + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 17 * MIB, "Store file too large");
    let encoded: BTreeMap<String, String> = serde_json::from_slice(&bytes)?;
    ensure!(encoded.len() <= 1024, "Store exceeds 1024 keys");
    let map = encoded
        .into_iter()
        .map(|(k, v)| Ok((k, STANDARD.decode(v)?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
    validate(&map)?;
    Ok(map)
}
pub fn write(path: &Path, map: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    validate(map)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing data directory"))?;
    std::fs::create_dir_all(parent)?;
    let tmp = path.with_extension("json.tmp");
    let encoded: BTreeMap<_, _> = map.iter().map(|(k, v)| (k, STANDARD.encode(v))).collect();
    std::fs::write(&tmp, serde_json::to_vec(&encoded)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
