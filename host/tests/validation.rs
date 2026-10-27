#![allow(clippy::unwrap_used)]
use dutchman_plugins::{manifest::Manifest, policy, storage};
use serde_json::json;
use std::collections::BTreeMap;
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!("../../examples/hello-panel/plugin.json")).unwrap()
}
#[test]
fn valid_manifests() {
    for text in [
        include_str!("../../examples/hello-panel/plugin.json"),
        include_str!("../../examples/pilot/plugin.json"),
        include_str!("../../examples/importer-fusion/plugin.json"),
        include_str!("../../examples/importer-inventor/plugin.json"),
        include_str!("../../examples/importer-solidworks/plugin.json"),
        include_str!("../../examples/importer-freecad/plugin.json"),
    ] {
        serde_json::from_str::<Manifest>(text)
            .unwrap()
            .validate()
            .unwrap();
    }
}
#[test]
fn unknown_fields_refused() {
    let mut v = manifest();
    v["surprise"] = json!(true);
    assert!(serde_json::from_value::<Manifest>(v).is_err());
}
#[test]
fn manifest_ranges_and_paths() {
    for (key, value) in [
        ("component", json!("../outside.wasm")),
        ("api", json!("0.2")),
        ("world", json!("other")),
        ("panel", json!(null)),
        ("formats", json!([])),
    ] {
        let mut v = manifest();
        v[key] = value;
        assert!(
            serde_json::from_value::<Manifest>(v)
                .unwrap()
                .validate()
                .is_err(),
            "{key}"
        );
    }
    for (key, value) in [
        ("memory_mib", 15),
        ("memory_mib", 257),
        ("stack_kib", 511),
        ("component_mib", 17),
        ("transaction_seconds", 121),
    ] {
        let mut v = manifest();
        v["limits"] = json!({key:value});
        assert!(
            serde_json::from_value::<Manifest>(v)
                .unwrap()
                .validate()
                .is_err(),
            "{key}"
        );
    }
}
#[test]
fn duplicate_entries_refused() {
    let mut v = manifest();
    let e = v["entries"][0].clone();
    v["entries"].as_array_mut().unwrap().push(e);
    assert!(
        serde_json::from_value::<Manifest>(v)
            .unwrap()
            .validate()
            .is_err()
    );
}
#[test]
fn unsupported_permissions_refused() {
    let mut v = manifest();
    v["permissions"]["net"] = json!([{"origin":"https://example.com"}]);
    assert!(
        serde_json::from_value::<Manifest>(v)
            .unwrap()
            .validate()
            .is_err()
    );
}
#[test]
fn deny_list_cannot_be_overridden() {
    for id in [
        "plugin.run",
        "edit.undo",
        "edit.redo",
        "edit.checkpoint",
        "edit.restore_checkpoint",
        "diagnostics.inspect",
        "app.close",
        "prefs.set",
        "file.export",
        "doc.open",
        "render.export",
        "render.start",
        "engine.execute",
        "solid.crash_test",
    ] {
        assert!(
            policy::check("hello", &[id.into()], true, id, &json!({})).is_err(),
            "{id}"
        );
    }
    assert!(
        policy::check(
            "hello",
            &["solid.box".into()],
            true,
            "solid.box",
            &json!({})
        )
        .is_ok()
    );
    assert!(policy::check("hello", &[], true, "solid.box", &json!({})).is_err());
    assert!(
        policy::check(
            "hello",
            &["solid.box".into()],
            false,
            "solid.box",
            &json!({})
        )
        .is_err()
    );
    assert!(
        policy::check(
            "hello",
            &["engine.describe".into()],
            false,
            "engine.describe",
            &json!({})
        )
        .is_ok()
    );
}
#[test]
fn metadata_namespaces() {
    let cmds = vec!["document.metadata.set".into()];
    for key in ["plugin.hello", "plugin.hello.foo"] {
        assert!(
            policy::check(
                "hello",
                &cmds,
                true,
                "document.metadata.set",
                &json!({"values":{key:"x"}})
            )
            .is_ok()
        );
    }
    for key in ["plugin.hello-other", "document", "plugin.other.key"] {
        assert!(
            policy::check(
                "hello",
                &cmds,
                true,
                "document.metadata.set",
                &json!({"values":{key:"x"}})
            )
            .is_err()
        );
    }
}
#[test]
fn json_caps() {
    assert!(policy::json(&format!("{}0{}", "[".repeat(65), "]".repeat(65))).is_err());
    assert!(policy::json(&serde_json::to_string(&"x".repeat(4 * 1024 * 1024)).unwrap()).is_err());
    assert!(policy::json(r#"{"size":12}"#).is_ok());
}
#[test]
fn store_caps_and_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plugin.json");
    let mut map = BTreeMap::new();
    map.insert("counter".into(), b"12".to_vec());
    storage::write(&path, &map).unwrap();
    assert_eq!(storage::read(&path).unwrap(), map);
    map.insert("../escape".into(), vec![]);
    assert!(storage::validate(&map).is_err());
    map.remove("../escape");
    map.insert("large".into(), vec![0; 1024 * 1024 + 1]);
    assert!(storage::validate(&map).is_err());
}

#[test]
fn unknown_command_and_importer_folder_access_refused() {
    let mut v = manifest();
    v["commands"] = json!(["Solid.Box"]);
    assert!(
        serde_json::from_value::<Manifest>(v)
            .unwrap()
            .validate()
            .is_err()
    );
    let mut v: serde_json::Value =
        serde_json::from_str(include_str!("../../examples/importer-fusion/plugin.json")).unwrap();
    v["reads_input_folder"] = json!(true);
    assert!(
        serde_json::from_value::<Manifest>(v)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[test]
fn t33_t35_physical_local_widget_approval() {
    let original: serde_json::Value =
        serde_json::from_str(include_str!("../../examples/print/plugin.json")).unwrap();
    let m: Manifest = serde_json::from_value(original.clone()).unwrap();
    m.validate().unwrap();
    let fingerprint = dutchman_plugins::manifest::approval(&m);
    for (pointer, value) in [
        ("/permissions/local/0/mesh", json!(false)),
        ("/permissions/local/0/port", json!(8731)),
        ("/entries/1/confirm/action", json!("Start machine")),
        ("/surfaces/2/kind", json!("panel")),
    ] {
        let mut v = original.clone();
        *v.pointer_mut(pointer).unwrap() = value;
        assert_ne!(
            fingerprint,
            dutchman_plugins::manifest::approval(&serde_json::from_value::<Manifest>(v).unwrap())
        );
    }
    let mut changed = m.clone();
    changed.version = "0.2.0".into();
    changed.component = Some("new.wasm".into());
    assert_eq!(fingerprint, dutchman_plugins::manifest::approval(&changed));
    for pointer in ["/entries/1/confirm", "/entries/1/machine_kind"] {
        let mut v = original.clone();
        *v.pointer_mut(pointer).unwrap() = json!(null);
        assert!(
            serde_json::from_value::<Manifest>(v)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut changed = m.clone();
    changed.surfaces.push(changed.surfaces[2].clone());
    assert!(changed.validate().is_err());
    let mut changed = m;
    changed.world = Some("command-plugin".into());
    changed.panel = None;
    assert!(changed.validate().is_err());
}
