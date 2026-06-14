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
fn help_lists_new_subcommands() {
    let out = Command::new(BIN).arg("--help").output().expect("run andro");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for cmd in ["install", "launch", "uninstall", "shell"] {
        assert!(stdout.contains(cmd), "help missing `{cmd}`");
    }
}

#[test]
fn help_lists_lifecycle_and_util_commands() {
    let out = Command::new(BIN).arg("--help").output().expect("run andro");
    let stdout = String::from_utf8_lossy(&out.stdout);
    for cmd in [
        "autoclean",
        "screenshot",
        "clear",
        "list",
        "open",
        "applog",
        "push",
        "pull",
        "reverse",
        "record",
        "test",
        "wait",
        "completions",
    ] {
        assert!(stdout.contains(cmd), "help missing `{cmd}`");
    }
}

#[test]
fn completions_emit_a_script() {
    let out = Command::new(BIN)
        .args(["completions", "bash"])
        .output()
        .expect("run andro");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("andro"),
        "completion script should mention andro"
    );
}

#[test]
fn autoclean_dry_run_on_missing_home_is_noop() {
    let out = Command::new(BIN)
        .args(["--home", "/tmp/andro-ac-none-xyz", "autoclean", "--dry-run"])
        .output()
        .expect("run andro");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("nothing to clean"));
}

#[test]
fn status_exits_3_when_not_running() {
    let out = Command::new(BIN)
        .args(["--home", "/tmp/andro-status-none-xyz", "status"])
        .output()
        .expect("run andro");
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn config_file_drives_defaults() {
    let home = "/tmp/andro-cfg-test-xyz";
    std::fs::create_dir_all(home).expect("mkdir");
    std::fs::write(format!("{home}/config.toml"), "profile = tv\napi = 34\n").expect("write cfg");
    let out = Command::new(BIN)
        .args(["--home", home, "doctor"])
        .output()
        .expect("run andro");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("system-images;android-34;android-tv;"),
        "config.toml should drive api+profile; got:\n{stdout}"
    );
    let _ = std::fs::remove_dir_all(home);
}

#[test]
fn doctor_reports_playstore_image_for_phone() {
    let out = Command::new(BIN)
        .args([
            "--home",
            "/tmp/andro-playstore-test",
            "--playstore",
            "doctor",
        ])
        .output()
        .expect("run andro");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("google_apis_playstore"),
        "doctor should show the Play Store image; got:\n{stdout}"
    );
}

#[test]
fn playstore_with_tv_is_rejected() {
    let out = Command::new(BIN)
        .args([
            "--home",
            "/tmp/andro-pstv-test",
            "--tv",
            "--playstore",
            "doctor",
        ])
        .output()
        .expect("run andro");
    assert!(!out.status.success(), "--playstore --tv must error");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Play Store"),
        "expected a Play Store/TV conflict message; got:\n{stderr}"
    );
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

/// Full `.aab` flow: also downloads bundletool, reads the device spec, builds and
/// installs the matched splits.
/// `ANDRO_E2E_AAB=/path/app.aab cargo test -- --ignored e2e`
#[test]
#[ignore = "needs macOS and ~2GB of SDK downloads + bundletool"]
fn e2e_run_aab() {
    let aab = std::env::var("ANDRO_E2E_AAB").expect("set ANDRO_E2E_AAB");
    let status = Command::new(BIN)
        .args(["run", &aab, "--no-launch"])
        .status()
        .expect("run andro");
    assert!(status.success());
}
