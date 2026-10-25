// SPDX-License-Identifier: Apache-2.0
// Geometry import path adapted from Dutchman document.rs; see ATTRIBUTION.md.
use dutchman_core::{Model, Report, Severity};
const FORMAT: &str = "solidcraft/23";
use serde_json::{Value, json};
const MAX_COMPONENT_NAME: usize = 128;
const COMPONENT: u64 = 1;
const OCCURRENCE: u64 = 2;
const FEATURE: u64 = 3;
const IDENTITY: [[f64; 4]; 4] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
pub(crate) fn import_design(model: &Model, step_blob: &str) -> Value {
    let component = component_name(&model.name);
    json!({
        "format": FORMAT,
        "name": model.name,
        "units": "mm",
        "params": [],
        "next_id": FEATURE + 1,
        "components": [{ "id": COMPONENT, "name": component, "parent": 0 }],
        "occurrences": [{
            "id": OCCURRENCE,
            "name": format!("{component}:1"),
            "component": COMPONENT,
            "parent": 0,
            "grounded": false,
            "transform": IDENTITY,
        }],
        "features": [import_feature(model, FEATURE, &feature_name(model), Some(COMPONENT), false, step_blob)],
    })
}

pub(crate) fn import_feature(
    model: &Model,
    id: u64,
    name: &str,
    component: Option<u64>,
    suppressed: bool,
    step_blob: &str,
) -> Value {
    let mut feature = json!({
        "id": id,
        "name": name,
        "type": "import",
        "suppressed": suppressed,
        "file": source_file(model),
        "step": "",
        "step_blob": step_blob,
    });
    if let (Some(component), Some(fields)) = (component, feature.as_object_mut()) {
        fields.insert("component".to_owned(), json!(component));
    }
    feature
}

pub(crate) fn component_name(design: &str) -> String {
    let name: String = design.trim().chars().take(MAX_COMPONENT_NAME).collect();
    if name.is_empty() {
        "Import".to_owned()
    } else {
        name
    }
}

pub(crate) fn feature_name(model: &Model) -> String {
    model
        .ir
        .model
        .bodies
        .first()
        .and_then(|body| body.name.as_deref())
        .filter(|name| !generic_name(name))
        .map_or_else(|| model.name.clone(), str::to_owned)
}

fn generic_name(name: &str) -> bool {
    name.trim().is_empty()
        || name.starts_with('(')
        || name.eq_ignore_ascii_case("import")
        || name.eq_ignore_ascii_case("unnamed")
}

fn source_file(model: &Model) -> String {
    model
        .source_file
        .clone()
        .unwrap_or_else(|| format!("{}.step", model.name))
}
pub(crate) fn report_unmapped(model: &Model, report: &mut Report) {
    let m = &model.ir.model;
    for (count, what) in [
        (m.features.len(), "feature history"),
        (m.sketches.len(), "sketches"),
        (m.parameters.len(), "parameters"),
    ] {
        if count > 0 {
            report.push(
                Severity::Degraded,
                what,
                format!("{count} carried as a non-parametric body, not as editable history"),
            );
        }
    }
}
