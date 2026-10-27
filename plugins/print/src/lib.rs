// SPDX-License-Identifier: MIT OR Apache-2.0
wit_bindgen::generate!({ path: "../../wit", world: "panel-plugin" });
use exports::solidcraft::plugin::{command, panel};
use serde_json::{Value, json};
use solidcraft::plugin::{host, net, store, types::PluginError as E};
struct Print;
fn invalid(s: impl ToString) -> E {
    E::Invalid(s.to_string())
}
fn load(key: &str) -> Value {
    store::get(key)
        .ok()
        .flatten()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null)
}
fn save(key: &str, value: &Value) -> Result<(), E> {
    store::set(key, &serde_json::to_vec(value).map_err(invalid)?)
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn api(path: &str, post: bool, mesh: bool) -> Result<Value, E> {
    let response = net::fetch(&net::Request {
        method: if post {
            net::Method::Post
        } else {
            net::Method::Get
        },
        url: format!("local:dutchman-print{path}"),
        headers: vec![],
        body: vec![],
        mesh: mesh.then_some(net::MeshBody {
            bodies: vec![],
            format: net::MeshFormat::ThreeMf,
            quality: net::MeshQuality::Standard,
        }),
    })?;
    let value = if response.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&response.body).map_err(invalid)?
    };
    if !(200..300).contains(&response.status) {
        return Err(E::Failed(
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Print service request failed")
                .into(),
        ));
    }
    Ok(value)
}
fn printers() -> Result<Value, E> {
    let value = api("/api/v1/printers", false, false)?;
    if !value.is_array() {
        return Err(invalid("Invalid printer list"));
    }
    save("printers", &value)?;
    let selected = load("printer");
    if !value
        .as_array()
        .is_some_and(|a| a.iter().any(|p| p["id"] == selected))
    {
        save(
            "printer",
            &value.get(0).map(|p| p["id"].clone()).unwrap_or(Value::Null),
        )?;
    }
    Ok(value)
}
fn selected(params: &Value) -> Result<String, E> {
    params
        .get("printer")
        .cloned()
        .unwrap_or_else(|| load("printer"))
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid("Choose a printer."))
}
fn execute(entry: &str, params: &Value) -> Result<Value, E> {
    if entry == "status" {
        return printers();
    }
    if load("printers").is_null() {
        printers()?;
    }
    let printer = if entry == "slice" {
        selected(params)?
    } else {
        params
            .get("printer")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| invalid("Physical action requires a printer parameter."))?
    };
    save("printer", &json!(printer))?;
    let p = encode(&printer);
    match entry {
        "slice" => {
            let mut path = format!("/api/v1/jobs?printer={p}");
            for key in ["printer_profile", "print_profile", "filament_profile"] {
                let value = params.get(key).cloned().unwrap_or_else(|| load(key));
                if let Some(value) = value.as_str().filter(|s| !s.is_empty()) {
                    path.push_str(&format!("&{key}={}", encode(value)));
                }
            }
            let job = api(&path, true, true)?;
            save("job", &job)?;
            Ok(job)
        }
        "print" | "cancel" => {
            let job = load("job");
            let id = params
                .get("job")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Prepare a job first."))?;
            if entry == "print"
                && (job["printer"].as_str() != Some(&printer) || job["id"].as_str() != Some(id))
            {
                return Err(invalid("Prepare a job for the selected printer."));
            }
            let action = if entry == "print" { "start" } else { "cancel" };
            let result = api(
                &format!("/api/v1/jobs/{}/{action}", encode(id)),
                true,
                false,
            )?;
            save("job", &result)?;
            Ok(result)
        }
        "pause" | "resume" | "clear" => api(&format!("/api/v1/printers/{p}/{entry}"), true, false),
        _ => Err(invalid("Unknown print entry")),
    }
}
fn message(error: E) -> String {
    match error {
        E::Failed(s) | E::Denied(s) | E::Invalid(s) | E::Limit(s) => s,
    }
}
impl command::Guest for Print {
    fn execute(entry: String, params: String) -> Result<String, E> {
        let params = serde_json::from_str(&params).map_err(invalid)?;
        let result = execute(&entry, &params);
        if let Err(error) = &result {
            save("error", &json!(message(error.clone())))?;
        } else {
            save("error", &Value::Null)?;
        }
        result.map(|v| v.to_string())
    }
}
fn button(id: &str, label: &str, enabled: bool) -> panel::Item {
    panel::Item::Button(panel::Button {
        id: id.into(),
        label: label.into(),
        enabled,
    })
}
fn text(s: String) -> panel::Item {
    panel::Item::Text(panel::Styled {
        text: s,
        tone: panel::Tone::Normal,
    })
}
fn busy(p: &Value) -> bool {
    matches!(p["state"].as_str(), Some("printing" | "paused"))
}
fn short(s: &str) -> String {
    let value: String = s.chars().filter(|c| !c.is_control()).take(40).collect();
    if value.is_empty() {
        "Printer".into()
    } else {
        value
    }
}
impl panel::Guest for Print {
    fn handle(event: panel::Event) -> Result<(), E> {
        let result = match event {
            panel::Event::Opened | panel::Event::Refresh => printers().map(|_| ()),
            panel::Event::Action(action) => (|| {
                for (key, value) in &action.inputs {
                    match value {
                        panel::Value::Selected(n) if key == "printer" => {
                            if let Some(p) = load("printers").get(*n as usize) {
                                save("printer", &p["id"])?;
                            }
                        }
                        panel::Value::Text(s)
                            if ["printer_profile", "print_profile", "filament_profile"]
                                .contains(&key.as_str()) =>
                        {
                            save(key, &json!(s))?
                        }
                        _ => {}
                    }
                }
                if ["print", "cancel", "pause", "resume", "clear"].contains(&action.id.as_str()) {
                    // Freeze the printer and job shown at the click into the confirmation params.
                    let job = load("job");
                    let printer = load("printer");
                    let fleet = load("printers");
                    let active_job = fleet
                        .as_array()
                        .and_then(|a| a.iter().find(|p| p["id"] == printer && busy(p)))
                        .map(|p| p["job_id"].clone());
                    let job_id = if action.id == "cancel" {
                        active_job.unwrap_or_else(|| job["id"].clone())
                    } else {
                        job["id"].clone()
                    };
                    host::request_execute(
                        &action.id,
                        &json!({"printer":load("printer"), "job":job_id}).to_string(),
                    )
                } else if action.id == "slice" {
                    execute("slice", &json!({})).map(|_| ())
                } else {
                    printers().map(|_| ())
                }
            })(),
            panel::Event::Executed(e) => match e.outcome {
                Ok(_) => printers().map(|_| ()),
                Err(e) => Err(E::Failed(e)),
            },
            _ => Ok(()),
        };
        save(
            "error",
            &result
                .err()
                .map(message)
                .map(Value::String)
                .unwrap_or(Value::Null),
        )?;
        Ok(())
    }
    fn render() -> Vec<panel::Item> {
        let fleet = load("printers");
        let fleet = fleet.as_array().cloned().unwrap_or_default();
        let selected = load("printer");
        let current = fleet.iter().find(|p| p["id"] == selected);
        let active = fleet.iter().filter(|p| busy(p)).max_by(|a, b| {
            (a["state"] == "printing")
                .cmp(&(b["state"] == "printing"))
                .then_with(|| {
                    a["progress"]
                        .as_f64()
                        .unwrap_or(0.0)
                        .total_cmp(&b["progress"].as_f64().unwrap_or(0.0))
                })
        });
        let _ = host::schedule_refresh(if active.is_some() { 5 } else { 30 });
        let (lines, progress, mut state) = if let Some(p) = active {
            (
                vec![
                    short(p["name"].as_str().unwrap_or("Printer")),
                    short(&format!(
                        "{} — {} s left",
                        p["state"].as_str().unwrap_or("busy"),
                        p["seconds_left"]
                            .as_u64()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "?".into())
                    )),
                ],
                p["progress"]
                    .as_f64()
                    .filter(|p| p.is_finite())
                    .map(|p| p.clamp(0.0, 1.0)),
                panel::WidgetState::Busy,
            )
        } else {
            (
                vec![format!("{} printers idle", fleet.len())],
                None,
                if fleet.iter().any(|p| p["needs_clearing"] == true) {
                    panel::WidgetState::Attention
                } else {
                    panel::WidgetState::Idle
                },
            )
        };
        if load("error").is_string() {
            state = panel::WidgetState::Error;
        }
        let mut items = vec![
            panel::Item::Heading("Print selected bodies".into()),
            panel::Item::Status(panel::Status {
                lines,
                progress,
                state,
            }),
        ];
        if let Some(error) = load("error").as_str() {
            items.push(text(error.chars().take(1024).collect()));
        }
        if !fleet.is_empty() {
            items.push(panel::Item::Choice(panel::Choice {
                id: "printer".into(),
                label: "Printer".into(),
                options: fleet
                    .iter()
                    .take(64)
                    .map(|p| p["name"].as_str().unwrap_or("Printer").into())
                    .collect(),
                selected: fleet
                    .iter()
                    .take(64)
                    .position(|p| p["id"] == selected)
                    .unwrap_or(0) as u32,
            }));
        }
        for key in ["printer_profile", "print_profile", "filament_profile"] {
            items.push(panel::Item::TextInput(panel::TextInput {
                id: key.into(),
                label: key.replace('_', " "),
                value: load(key).as_str().unwrap_or("").into(),
                multiline: false,
                max_chars: 256,
            }));
        }
        let job = load("job");
        if !job.is_null() {
            items.push(text(format!("Estimate: {}", job["estimate"])));
        }
        items.push(button("slice", "Slice selection", current.is_some()));
        items.push(button(
            "print",
            "Print",
            job["state"] == "ready" && job["printer"] == selected,
        ));
        if let Some(p) = current {
            items.push(button("pause", "Pause", p["state"] == "printing"));
            items.push(button("resume", "Resume", p["state"] == "paused"));
            items.push(button(
                "cancel",
                "Cancel",
                busy(p) || job["state"] == "ready" && job["printer"] == selected,
            ));
            if p["needs_clearing"] == true {
                items.push(button("clear", "Bed is clear", p["state"] == "idle"));
            }
        }
        for p in fleet.iter().take(100) {
            let progress = p["progress"]
                .as_f64()
                .map(|n| format!("{:.0}", n.clamp(0.0, 1.0) * 100.0))
                .unwrap_or_else(|| "?".into());
            let seconds = p["seconds_left"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into());
            items.push(text(format!(
                "{}: {} — {progress}% — {seconds} s left",
                p["name"].as_str().unwrap_or("Printer"),
                p["state"].as_str().unwrap_or("unknown")
            )));
        }
        items
    }
    fn markers() -> Vec<panel::Marker> {
        vec![]
    }
}
#[allow(unsafe_code)]
mod component_exports {
    use super::Print;
    super::export!(Print with_types_in crate);
}
