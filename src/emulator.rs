//! Pure helpers for talking to the emulator over adb.
//!
//! These parse the textual output of `adb` commands. The orchestration that
//! actually runs them lives in `commands.rs`; keeping the parsing pure makes it
//! unit-testable without a running emulator.

use std::path::Path;

/// How `run`/`install` should hand an install target to adb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallInput {
    /// A single `.apk` → `adb install`.
    Single,
    /// A directory of split `.apk`s → `adb install-multiple`.
    MultiApk,
    /// A `.xapk`/`.apks`/`.apkm` zip of split apks → unzip then install-multiple.
    Bundle,
}

/// Classify an install target. `is_dir` is the filesystem fact about `path`,
/// passed in so this stays pure and unit-testable.
pub fn classify_install_input(path: &Path, is_dir: bool) -> InstallInput {
    if is_dir {
        return InstallInput::MultiApk;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xapk" | "apks" | "apkm" => InstallInput::Bundle,
        _ => InstallInput::Single,
    }
}

/// Parse the output of `adb shell pm list packages` into bare package names.
///
/// Each line looks like `package:com.example.app`; anything else is ignored.
pub fn parse_packages(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .filter(|pkg| !pkg.is_empty())
        .map(|pkg| pkg.to_string())
        .collect()
}

/// Packages present in `after` but not in `before` — i.e. what an install added.
pub fn newly_installed(before: &[String], after: &[String]) -> Vec<String> {
    after
        .iter()
        .filter(|pkg| !before.contains(pkg))
        .cloned()
        .collect()
}

/// True when `getprop sys.boot_completed` reports the emulator has finished booting.
pub fn boot_completed(getprop_output: &str) -> bool {
    getprop_output.trim() == "1"
}

/// True once PackageManager actually answers `adb shell cmd package list
/// packages` — i.e. it lists packages and shows no "service unavailable" error.
///
/// `sys.boot_completed=1` fires before the package service is fully up, so
/// installing immediately after boot can flake; this is the readiness gate.
pub fn package_manager_ready(cmd_package_output: &str) -> bool {
    let lower = cmd_package_output.to_ascii_lowercase();
    if lower.contains("can't find service")
        || lower.contains("cannot find service")
        || lower.contains("failure")
        || lower.contains("error:")
    {
        return false;
    }
    // A working PackageManager lists at least the system packages.
    cmd_package_output
        .lines()
        .any(|l| l.trim_start().starts_with("package:"))
}

/// Extract the `pkg/activity` component from `cmd package resolve-activity
/// --brief -c <category> <pkg>` output. `None` if nothing resolved.
pub fn parse_resolved_activity(output: &str, pkg: &str) -> Option<String> {
    let prefix = format!("{pkg}/");
    output
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with(&prefix))
        .map(|l| l.to_string())
}

// ---- autoclean pure helpers -------------------------------------------------

/// Human-readable byte size (base-1024), e.g. `0 B`, `16.0 KB`, `2.8 GB`.
pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", UNITS[i])
}

/// Extract the AVD's `image.sysdir.1=<path>` value from a `config.ini`, trimmed
/// of whitespace and any trailing slash. `None` if the key is absent.
pub fn parse_image_sysdir(config_ini: &str) -> Option<String> {
    config_ini
        .lines()
        .find_map(|l| l.trim().strip_prefix("image.sysdir.1="))
        .map(|v| v.trim().trim_end_matches('/').to_string())
}

/// System images that are installed but referenced by no live AVD — i.e. safe
/// to prune. Both lists are relative image paths (`system-images/<api>/<tag>/<abi>`).
pub fn unreferenced_images(installed: &[String], referenced: &[String]) -> Vec<String> {
    installed
        .iter()
        .filter(|img| !referenced.contains(img))
        .cloned()
        .collect()
}

/// True when every entry in `~/.android` is an andro-attributable artifact, so
/// the leftover residue is safe to remove. False if ANY foreign file is present
/// (e.g. an Android Studio `avd/`, `debug.keystore`, `ddms.cfg`) — never touch a
/// shared `~/.android` that holds something we didn't create.
pub fn android_residue_is_andro_only(entries: &[String]) -> bool {
    !entries.is_empty() && entries.iter().all(|e| is_andro_artifact(e))
}

fn is_andro_artifact(name: &str) -> bool {
    const KNOWN: [&str; 6] = [
        "adbkey",
        "adbkey.pub",
        "cache",
        "emu-update-last-check.ini",
        "emu-last-feature-flags.protobuf",
        "userid",
    ];
    KNOWN.contains(&name) || name.starts_with("modem-nv-ram") || name.starts_with("emu-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_scales_units() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KB");
        assert_eq!(human_bytes(16 * 1024), "16.0 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024 + 512 * 1024), "3.5 MB");
        assert_eq!(human_bytes(3_006_477_107), "2.8 GB");
    }

    #[test]
    fn parse_image_sysdir_extracts_and_trims() {
        let ini = "avd.ini.displayname=andro tv\n\
                   image.sysdir.1=system-images/android-36/android-tv/arm64-v8a/\n\
                   tag.id=android-tv\n";
        assert_eq!(
            parse_image_sysdir(ini),
            Some("system-images/android-36/android-tv/arm64-v8a".to_string())
        );
    }

    #[test]
    fn parse_image_sysdir_none_when_absent() {
        assert_eq!(parse_image_sysdir("hw.ramSize=2048\n"), None);
        assert_eq!(parse_image_sysdir(""), None);
    }

    #[test]
    fn unreferenced_images_is_set_difference() {
        let installed = vec![
            "system-images/android-34/google_apis/arm64-v8a".to_string(),
            "system-images/android-36/android-tv/arm64-v8a".to_string(),
        ];
        let referenced = vec!["system-images/android-36/android-tv/arm64-v8a".to_string()];
        assert_eq!(
            unreferenced_images(&installed, &referenced),
            vec!["system-images/android-34/google_apis/arm64-v8a"]
        );
    }

    #[test]
    fn unreferenced_images_empty_when_all_in_use() {
        let imgs = vec!["system-images/android-36/android-tv/arm64-v8a".to_string()];
        assert!(unreferenced_images(&imgs, &imgs).is_empty());
    }

    #[test]
    fn android_residue_andro_only_true_for_known_set() {
        let entries = vec![
            "adbkey".to_string(),
            "adbkey.pub".to_string(),
            "cache".to_string(),
            "emu-update-last-check.ini".to_string(),
            "emu-last-feature-flags.protobuf".to_string(),
            "modem-nv-ram-5554".to_string(),
            "userid".to_string(),
        ];
        assert!(android_residue_is_andro_only(&entries));
    }

    #[test]
    fn android_residue_false_when_foreign_present() {
        // an Android Studio AVD dir, a keystore, ddms config → never touch it
        assert!(!android_residue_is_andro_only(&[
            "adbkey".into(),
            "avd".into()
        ]));
        assert!(!android_residue_is_andro_only(&["debug.keystore".into()]));
        assert!(!android_residue_is_andro_only(&[
            "adbkey".into(),
            "ddms.cfg".into()
        ]));
    }

    #[test]
    fn android_residue_false_when_empty() {
        assert!(!android_residue_is_andro_only(&[]));
    }

    #[test]
    fn classify_single_apk() {
        assert_eq!(
            classify_install_input(Path::new("app.apk"), false),
            InstallInput::Single
        );
        assert_eq!(
            classify_install_input(Path::new("/x/App.APK"), false),
            InstallInput::Single
        );
    }

    #[test]
    fn classify_directory_is_multiapk() {
        assert_eq!(
            classify_install_input(Path::new("splitdir"), true),
            InstallInput::MultiApk
        );
    }

    #[test]
    fn classify_bundles_by_extension() {
        for f in ["a.xapk", "a.apks", "a.apkm", "A.XAPK"] {
            assert_eq!(
                classify_install_input(Path::new(f), false),
                InstallInput::Bundle,
                "{f} should be a bundle"
            );
        }
    }

    #[test]
    fn classify_unknown_extension_is_single() {
        assert_eq!(
            classify_install_input(Path::new("weird.bin"), false),
            InstallInput::Single
        );
    }

    #[test]
    fn parse_packages_strips_prefix_and_blank_lines() {
        let out = "package:com.android.settings\npackage:com.example.app\n\n";
        assert_eq!(
            parse_packages(out),
            vec!["com.android.settings", "com.example.app"]
        );
    }

    #[test]
    fn parse_packages_tolerates_carriage_returns_and_noise() {
        // adb output often yields CRLF; non-package lines are dropped.
        let out = "package:com.a\r\nWARNING: something\r\npackage:com.b\r\n";
        assert_eq!(parse_packages(out), vec!["com.a", "com.b"]);
    }

    #[test]
    fn newly_installed_is_the_set_difference() {
        let before = vec!["com.a".to_string(), "com.b".to_string()];
        let after = vec![
            "com.a".to_string(),
            "com.b".to_string(),
            "com.new".to_string(),
        ];
        assert_eq!(newly_installed(&before, &after), vec!["com.new"]);
    }

    #[test]
    fn newly_installed_empty_when_nothing_added() {
        let pkgs = vec!["com.a".to_string()];
        assert!(newly_installed(&pkgs, &pkgs).is_empty());
    }

    #[test]
    fn boot_completed_true_only_for_one() {
        assert!(boot_completed("1\n"));
        assert!(boot_completed("  1  "));
        assert!(!boot_completed("0\n"));
        assert!(!boot_completed(""));
        assert!(!boot_completed("error: device offline"));
    }

    #[test]
    fn package_manager_ready_true_when_packages_listed() {
        let out = "package:android\r\npackage:com.android.shell\r\n";
        assert!(package_manager_ready(out));
    }

    #[test]
    fn package_manager_ready_false_when_service_missing() {
        // pm not up yet — `cmd`/`am` print this before PackageManager registers.
        assert!(!package_manager_ready("Error: Can't find service: package"));
        assert!(!package_manager_ready("cmd: Can't find service: package"));
    }

    #[test]
    fn package_manager_ready_false_on_empty_or_failure() {
        assert!(!package_manager_ready(""));
        assert!(!package_manager_ready("   \n"));
        assert!(!package_manager_ready("Failure [INSTALL_FAILED]"));
    }

    #[test]
    fn resolved_activity_extracts_component() {
        let out = "priority=0 preferredOrder=0 match=0x108000 isDefault=true\r\n\
                   org.drive_hunter/xyz.netfly.SplashActivity\r\n";
        assert_eq!(
            parse_resolved_activity(out, "org.drive_hunter"),
            Some("org.drive_hunter/xyz.netfly.SplashActivity".to_string())
        );
    }

    #[test]
    fn resolved_activity_none_when_absent() {
        assert_eq!(parse_resolved_activity("No activity found", "com.x"), None);
        assert_eq!(parse_resolved_activity("", "com.x"), None);
        // a different package's line must not match
        assert_eq!(parse_resolved_activity("other.pkg/Act", "com.x"), None);
    }
}
