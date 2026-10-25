use crate::manifest::MIB;
use anyhow::{Result, ensure};
use serde_json::Value;
pub fn json(text: &str) -> Result<Value> {
    ensure!(text.len() <= 4 * MIB, "JSON exceeds 4 MiB");
    let v: Value = serde_json::from_str(text)?;
    fn depth(v: &Value, d: usize) -> bool {
        d <= 64
            && match v {
                Value::Array(a) => a.iter().all(|v| depth(v, d + 1)),
                Value::Object(a) => a.values().all(|v| depth(v, d + 1)),
                _ => true,
            }
    }
    ensure!(depth(&v, 0), "JSON exceeds depth 64");
    Ok(v)
}
pub fn denied(id: &str) -> bool {
    id.contains("crash")
        || ["plugin.", "diagnostics.", "app.", "prefs.", "file.", "doc."]
            .iter()
            .any(|p| id.starts_with(p))
        || (id.starts_with("engine.") && id != "engine.describe")
        || [
            "edit.undo",
            "edit.redo",
            "edit.checkpoint",
            "edit.restore_checkpoint",
            "render.export",
            "render.start",
        ]
        .contains(&id)
}
pub fn check(
    plugin: &str,
    listed: &[String],
    transaction: bool,
    id: &str,
    args: &Value,
) -> Result<()> {
    ensure!(
        listed.iter().any(|c| c == id) && !denied(id),
        "Command denied: {id}"
    );
    const SAFE: &[&str] = &[
        "document.inspect",
        "document.metadata",
        "model.faces",
        "model.edges",
        "engine.describe",
        "select.set",
        "select.clear",
        "view.apply",
    ];
    ensure!(
        transaction || SAFE.contains(&id),
        "Command is not handler-safe: {id}"
    );
    if ["document.metadata.set", "document.metadata.remove"].contains(&id) {
        let prefix = format!("plugin.{plugin}");
        let valid = |s: &str| s == prefix || s.starts_with(&format!("{prefix}."));
        if let Some(values) = args.get("values").and_then(Value::as_object) {
            ensure!(
                values.keys().all(|k| valid(k)),
                "Metadata key outside plugin namespace"
            );
        } else {
            ensure!(
                args.get("key").and_then(Value::as_str).is_some_and(valid),
                "Metadata key outside plugin namespace"
            );
        }
    }
    Ok(())
}
