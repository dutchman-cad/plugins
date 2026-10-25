// SPDX-License-Identifier: MIT OR Apache-2.0
wit_bindgen::generate!({ path: "../../wit", world: "command-plugin" });
use exports::solidcraft::plugin::command::Guest;
use sc_pilot::{catalog::Catalog, planner};
use serde_json::{Value, json};
use solidcraft::plugin::types::PluginError;
struct Pilot;
impl Guest for Pilot {
    fn execute(entry: String, params: String) -> Result<String, PluginError> {
        let value: Value =
            serde_json::from_str(&params).map_err(|e| PluginError::Invalid(e.to_string()))?;
        let request = value
            .as_str()
            .or_else(|| value.get("request").and_then(Value::as_str))
            .ok_or_else(|| {
                PluginError::Invalid("Pass a JSON string or {request: string}".into())
            })?;
        let catalog = Catalog::bundled().map_err(PluginError::Failed)?;
        let result = match entry.as_str() {
            "how" => serde_json::to_value(catalog.how(request).map_err(PluginError::Invalid)?),
            "plan" => {
                let input = json!({"messages":[{"role":"user","content":request}]});
                let plan = planner::plan(&input, &catalog).map_err(PluginError::Invalid)?;
                Ok(match plan {
                    Some(p) => json!({"rule":p.rule,"commands":p.calls}),
                    None => json!({"message":"No exact plan; inspect these catalogue matches before executing.","matches":catalog.how(request).map_err(PluginError::Invalid)?}),
                })
            }
            _ => return Err(PluginError::Invalid("Unknown entry".into())),
        }.map_err(|e| PluginError::Failed(e.to_string()))?;
        Ok(result.to_string())
    }
}
// Only the binding generator's ABI trampolines need unsafe code.
#[allow(unsafe_code)]
mod component_exports {
    use super::Pilot;
    super::export!(Pilot with_types_in crate);
}
