// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used)]
#![cfg(target_os = "linux")]

/// Runs the component integration suite after build-plugins.sh installs examples.
#[test]
#[ignore = "Requires the built example components"]
fn offline_components() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let status = std::process::Command::new("python3")
        .arg("tests/smoke.py")
        .current_dir(root)
        .env(
            "DUTCHMAN_PLUGINS_BIN",
            env!("CARGO_BIN_EXE_dutchman-plugins"),
        )
        .status()
        .unwrap();
    assert!(status.success(), "Component smoke tests failed");
}
