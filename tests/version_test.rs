//! Verifies basic CLI execution and version reporting.

use std::path::PathBuf;
use std::process::Command;

fn ox_bin() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_ox") {
        return PathBuf::from(path);
    }
    // Determine path relative to test runner executable in target/debug/deps/
    let mut path = std::env::current_exe().expect("failed to get current_exe");
    path.pop(); // remove test binary name
    if path.file_name().and_then(|n| n.to_str()) == Some("deps") {
        path.pop(); // up from deps to target/debug
    }
    path.push(if cfg!(windows) { "ox.exe" } else { "ox" });
    path
}

#[test]
fn test_ox_version() {
    let bin = ox_bin();
    let mut cmd = Command::new(&bin);
    cmd.arg("--version");
    let output = cmd.output().expect("failed to execute ox binary");
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ox 0.1.0"));
}

#[test]
fn test_ox_help() {
    let bin = ox_bin();
    let mut cmd = Command::new(&bin);
    cmd.arg("--help");
    let output = cmd.output().expect("failed to execute ox binary");
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Oxidize"));
    assert!(stdout.contains("hash-object"));
    assert!(stdout.contains("init"));
    assert!(stdout.contains("commit"));
}
