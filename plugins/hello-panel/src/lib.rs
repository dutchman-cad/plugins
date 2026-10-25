// SPDX-License-Identifier: MIT OR Apache-2.0
wit_bindgen::generate!({ path: "../../wit", world: "panel-plugin" });
use exports::solidcraft::plugin::{command, panel};
use serde_json::{Value, json};
use solidcraft::plugin::{engine, host, store, types::PluginError};
struct Hello;
fn size() -> f64 {
    store::get("size")
        .ok()
        .flatten()
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0)
}
impl command::Guest for Hello {
    fn execute(entry: String, params: String) -> Result<String, PluginError> {
        if entry != "box" {
            return Err(PluginError::Invalid("Unknown entry".into()));
        }
        let p: Value =
            serde_json::from_str(&params).map_err(|e| PluginError::Invalid(e.to_string()))?;
        let n = p.get("size").and_then(Value::as_f64).unwrap_or(10.0);
        if !n.is_finite() || !(1.0..=1000.0).contains(&n) {
            return Err(PluginError::Invalid("Size must be 1–1000 mm".into()));
        }
        engine::run(
            "solid.box",
            &json!({"length":n,"width":n,"height":n}).to_string(),
        )
    }
}
impl panel::Guest for Hello {
    fn handle(e: panel::Event) -> Result<(), PluginError> {
        match e {
            panel::Event::Action(a) => {
                let n = a
                    .inputs
                    .iter()
                    .find_map(|(id, v)| match v {
                        panel::Value::Number(n) if id == "size" => Some(*n),
                        _ => None,
                    })
                    .unwrap_or_else(size);
                if !n.is_finite() || !(1.0..=1000.0).contains(&n) {
                    return Err(PluginError::Invalid("Size must be 1–1000 mm".into()));
                }
                store::set("size", n.to_string().as_bytes())?;
                if a.id == "make-box" {
                    host::request_execute("box", &json!({"size":n}).to_string())?;
                }
            }
            panel::Event::Executed(e) if e.outcome.is_ok() => {
                let count = store::get("count")?
                    .and_then(|v| String::from_utf8(v).ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                store::set("count", count.saturating_add(1).to_string().as_bytes())?;
            }
            _ => {}
        }
        Ok(())
    }
    fn render() -> Vec<panel::Item> {
        let count = store::get("count")
            .ok()
            .flatten()
            .and_then(|v| String::from_utf8(v).ok())
            .unwrap_or_else(|| "0".into());
        vec![
            panel::Item::Heading(format!("Hello — {count} boxes")),
            panel::Item::NumberInput(panel::NumberInput {
                id: "size".into(),
                label: "Size".into(),
                value: size(),
                min: 1.0,
                max: 1000.0,
                step: 1.0,
                unit: "mm".into(),
            }),
            panel::Item::Button(panel::Button {
                id: "make-box".into(),
                label: "Make box".into(),
                enabled: true,
            }),
        ]
    }
    fn markers() -> Vec<panel::Marker> {
        vec![]
    }
}
// Only the binding generator's ABI trampolines need unsafe code.
#[allow(unsafe_code)]
mod component_exports {
    use super::Hello;
    super::export!(Hello with_types_in crate);
}
