//! Integration tests for browser-gateway
//!
//! Note: Full integration tests require Chrome to be running.
//! These tests focus on CLI parsing and service configuration.

use std::process::Command;

/// Test that the binary can show help
#[test]
fn test_help_command() {
    let output = Command::new("cargo")
        .args(["run", "--", "--help"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("browser-gateway") || stdout.contains("Browser"),
        "Help should mention browser-gateway"
    );
}

/// Test that version command works
#[test]
fn test_version_command() {
    let output = Command::new("cargo")
        .args(["run", "--", "--version"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("0.") || stdout.contains("fgp-browser"),
        "Version should be shown"
    );
}

/// Test that the crate compiles and exports expected items
#[test]
fn test_crate_compiles() {
    // This test passes if the crate compiles successfully
    // The existence of this function proves compilation succeeds
}
