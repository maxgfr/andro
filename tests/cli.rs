//! Integration tests for the `andro` binary.
//!
//! Hermetic tests use a throwaway `--home` so they never touch a real `~/.andro`
//! or download anything. The full provision/boot/install path needs ~2GB of
//! downloads and a Mac, so it lives behind `#[ignore]` (see `e2e_*`).

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_andro");

#[test]
fn help_lists_core_commands() {
    let out = Command::new(BIN).arg("--help").output().expect("run andro");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for cmd in ["run", "up", "clean", "status", "doctor", "logcat"] {
        assert!(stdout.contains(cmd), "help missing `{cmd}`");
    }
    assert!(stdout.contains("--tv"), "help missing --tv flag");
}

#[test]
fn clean_on_missing_home_is_noop() {
    let out = Command::new(BIN)
        .args(["--home", "/tmp/andro-does-not-exist-xyz", "clean", "--yes"])
        .output()
        .expect("run andro");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("nothing to clean"));
}

#[test]
fn doctor_reports_image_for_tv_profile() {
    let out = Command::new(BIN)
        .args([
            "--home",
            "/tmp/andro-doctor-test",
            "--tv",
            "--api",
            "34",
            "doctor",
        ])
        .output()
        .expect("run andro");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("system-images;android-34;android-tv;"),
        "doctor should show the TV image; got:\n{stdout}"
    );
}

/// Full flow. Requires macOS + ~2GB of downloads on first run.
/// `ANDRO_E2E_APK=/path/app.apk cargo test -- --ignored e2e`
#[test]
#[ignore = "needs macOS and ~2GB of SDK downloads"]
fn e2e_run_apk() {
    let apk = std::env::var("ANDRO_E2E_APK").expect("set ANDRO_E2E_APK");
    let status = Command::new(BIN)
        .args(["run", &apk, "--no-launch"])
        .status()
        .expect("run andro");
    assert!(status.success());
}
