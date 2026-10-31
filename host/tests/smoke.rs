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

/// Focused terminal rendering and confirmation coverage, using loopback fakes only.
#[test]
#[ignore = "Requires the built hello-panel and Print components"]
fn terminal_panels() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let status = std::process::Command::new("python3")
        .args([
            "-m",
            "unittest",
            "smoke.Smoke.test_panel_control_envelope",
            "print_api.PrintSmoke.test_named_panel_controls_preserve_confirmation",
            "print_api.PrintSmoke.test_overview_select_changes_selection_and_sends_no_action",
            "print_api.PrintSmoke.test_monitoring_only_has_no_machine_buttons_or_challenges",
        ])
        .current_dir(root.join("tests"))
        .env(
            "DUTCHMAN_PLUGINS_BIN",
            env!("CARGO_BIN_EXE_dutchman-plugins"),
        )
        .status()
        .unwrap();
    assert!(status.success(), "Terminal panel tests failed");
}
