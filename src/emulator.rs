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
    /// An `.aab` Android App Bundle → bundletool builds device-matched splits,
    /// then install-multiple.
    Aab,
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
        "aab" => InstallInput::Aab,
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

/// Read a `key=value` out of a `config.ini`, trimmed. `None` when absent.
pub fn config_value(config_ini: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    config_ini
        .lines()
        .find_map(|l| l.trim().strip_prefix(&prefix))
        .map(|v| v.trim().to_string())
}

/// True when an existing AVD's `config.ini` still matches the requested system
/// image and device profile.
///
/// `expected_sysdir` is the image in path form (`system-images/android-36/...`,
/// i.e. `Config::image()` with `;` replaced by `/`) and `expected_device` the
/// avdmanager device name. A `config.ini` missing either key counts as a
/// mismatch: we cannot prove it matches, and an AVD is disposable by design.
pub fn avd_matches(config_ini: &str, expected_sysdir: &str, expected_device: &str) -> bool {
    let sysdir = parse_image_sysdir(config_ini);
    let device = config_value(config_ini, "hw.device.name");
    match (sysdir, device) {
        (Some(s), Some(d)) => s == expected_sysdir.trim_end_matches('/') && d == expected_device,
        _ => false,
    }
}

/// What an existing AVD is, in the same shape [`avd_label`] renders a request:
/// `android-34/google_apis/arm64-v8a on pixel`. Lets a recreate or refusal
/// message name exactly what changed — API level, image tag or device.
pub fn avd_label_from_config(config_ini: &str) -> String {
    let image = parse_image_sysdir(config_ini).unwrap_or_else(|| "unknown".to_string());
    let device = config_value(config_ini, "hw.device.name").unwrap_or_else(|| "unknown".into());
    avd_label(&image, &device)
}

/// The same label for a *requested* configuration. `sysdir` is the image in path
/// form (`Config::image()` with `;` replaced by `/`); the `system-images/` prefix
/// is dropped so both sides read as `android-36/google_apis/arm64-v8a on pixel`.
pub fn avd_label(sysdir: &str, device: &str) -> String {
    let image = sysdir
        .trim_end_matches('/')
        .strip_prefix("system-images/")
        .unwrap_or(sysdir.trim_end_matches('/'));
    format!("{image} on {device}")
}

/// The candidate package with the most recent `lastUpdateTime` in
/// `adb shell dumpsys package packages` output.
///
/// Reinstalling an app the emulator already has adds nothing to `pm list
/// packages`, so the install/launch path has no new package to point at. This
/// finds the one that was just written instead of guessing. `candidates` is the
/// third-party package list; anything outside it is ignored. `lastUpdateTime`
/// is `YYYY-MM-DD HH:MM:SS`, which compares correctly as a string.
pub fn most_recently_updated(dumpsys_packages: &str, candidates: &[String]) -> Option<String> {
    let mut current: Option<&str> = None;
    let mut best: Option<(String, String)> = None;
    for line in dumpsys_packages.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Package [") {
            current = rest.split(']').next();
        } else if let Some(value) = line.strip_prefix("lastUpdateTime=")
            && let Some(pkg) = current
            && candidates.iter().any(|c| c == pkg)
        {
            let when = value.trim().to_string();
            if best.as_ref().is_none_or(|(best_when, _)| when > *best_when) {
                best = Some((when, pkg.to_string()));
            }
        }
    }
    best.map(|(_, pkg)| pkg)
}

/// Parse `adb emu avd name`, whose reply is the AVD name followed by adb's
/// trailing `OK`. `None` when nothing is attached (`error: no emulator
/// detected`) or the console answered anything but a name.
pub fn parse_avd_name(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .filter(|l| {
            *l != "OK"
                && *l != "KO"
                && !l.starts_with("error")
                && !l.starts_with("KO:")
                && !l.contains(':')
        })
        .map(str::to_string)
}

/// Ensure an AVD `config.ini` enables the emulated hardware keyboard so the host
/// (Mac) keyboard types into the guest. avdmanager's device profiles default
/// `hw.keyboard=no`, which silently drops host key events — you tap a field, the
/// on-screen keyboard shows, but typing on the Mac does nothing. Flipping it to
/// `yes` forwards the physical keyboard; the soft IME still appears because the
/// system images default `show_ime_with_hard_keyboard=1`.
///
/// Pure string transform: returns the rewritten file when a change is needed, or
/// `None` if it already reads `hw.keyboard=yes` (so the caller can skip writing).
/// Only the exact `hw.keyboard=` key is touched — `hw.keyboard.charmap`/`.lid`
/// are left alone.
pub fn with_hw_keyboard_enabled(config_ini: &str) -> Option<String> {
    fn is_hw_keyboard(line: &str) -> bool {
        line.trim_start()
            .strip_prefix("hw.keyboard")
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    }
    let mut found = false;
    let mut changed = false;
    let mut out: Vec<String> = Vec::new();
    for line in config_ini.lines() {
        if is_hw_keyboard(line) {
            found = true;
            let value = line.split_once('=').map_or("", |(_, v)| v.trim());
            if value == "yes" {
                out.push(line.to_string());
            } else {
                out.push("hw.keyboard=yes".to_string());
                changed = true;
            }
        } else {
            out.push(line.to_string());
        }
    }
    if !found {
        out.push("hw.keyboard=yes".to_string());
        changed = true;
    }
    if !changed {
        return None;
    }
    let mut result = out.join("\n");
    if config_ini.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
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

    const AVD_INI: &str = "avd.ini.displayname=andro\n\
                           abi.type=arm64-v8a\n\
                           hw.device.name=pixel\n\
                           image.sysdir.1=system-images/android-36/google_apis/arm64-v8a/\n\
                           tag.id=google_apis\n";

    #[test]
    fn avd_matches_when_image_and_device_agree() {
        assert!(avd_matches(
            AVD_INI,
            "system-images/android-36/google_apis/arm64-v8a",
            "pixel"
        ));
    }

    #[test]
    fn avd_matches_false_on_different_api_or_device() {
        // a different API level (the common `--api 34` -> `--api 36` case)
        assert!(!avd_matches(
            AVD_INI,
            "system-images/android-34/google_apis/arm64-v8a",
            "pixel"
        ));
        // a different device profile
        assert!(!avd_matches(
            AVD_INI,
            "system-images/android-36/google_apis/arm64-v8a",
            "pixel_7"
        ));
        // a different tag (phone -> playstore image)
        assert!(!avd_matches(
            AVD_INI,
            "system-images/android-36/google_apis_playstore/arm64-v8a",
            "pixel"
        ));
    }

    #[test]
    fn avd_matches_false_when_keys_are_missing() {
        assert!(!avd_matches(
            "hw.ramSize=2048\n",
            "system-images/x",
            "pixel"
        ));
        assert!(!avd_matches(
            "image.sysdir.1=system-images/x\n",
            "system-images/x",
            "pixel"
        ));
        assert!(!avd_matches(
            "hw.device.name=pixel\n",
            "system-images/x",
            "pixel"
        ));
        assert!(!avd_matches("", "system-images/x", "pixel"));
    }

    const DUMPSYS: &str = "\
Packages:
  Package [com.android.settings] (a1b2c3):
    userId=1000
    firstInstallTime=2026-01-01 09:00:00
    lastUpdateTime=2026-09-04 18:00:00
  Package [com.old.app] (d4e5f6):
    userId=10201
    firstInstallTime=2026-02-02 10:00:00
    lastUpdateTime=2026-02-02 10:00:00
  Package [com.fresh.app] (7a8b9c):
    userId=10202
    firstInstallTime=2026-03-03 11:00:00
    lastUpdateTime=2026-09-04 17:59:12
";

    #[test]
    fn most_recently_updated_picks_the_latest_candidate() {
        let candidates = vec!["com.old.app".to_string(), "com.fresh.app".to_string()];
        assert_eq!(
            most_recently_updated(DUMPSYS, &candidates),
            Some("com.fresh.app".to_string())
        );
    }

    #[test]
    fn most_recently_updated_ignores_packages_outside_candidates() {
        // com.android.settings has the newest timestamp but is not third-party.
        let candidates = vec!["com.old.app".to_string()];
        assert_eq!(
            most_recently_updated(DUMPSYS, &candidates),
            Some("com.old.app".to_string())
        );
    }

    #[test]
    fn most_recently_updated_none_without_a_match() {
        assert_eq!(most_recently_updated("", &["com.a".to_string()]), None);
        assert_eq!(most_recently_updated(DUMPSYS, &[]), None);
        assert_eq!(
            most_recently_updated(DUMPSYS, &["com.absent.app".to_string()]),
            None
        );
    }

    #[test]
    fn parse_avd_name_reads_the_console_reply() {
        assert_eq!(
            parse_avd_name("andro-tv\r\nOK\r\n"),
            Some("andro-tv".to_string())
        );
        assert_eq!(parse_avd_name("andro\nOK\n"), Some("andro".to_string()));
        assert_eq!(parse_avd_name("\n\nandro\nOK\n"), Some("andro".to_string()));
    }

    #[test]
    fn parse_avd_name_none_when_no_emulator() {
        assert_eq!(parse_avd_name("error: no emulator detected"), None);
        assert_eq!(parse_avd_name("KO: unknown command\r\n"), None);
        assert_eq!(parse_avd_name("OK\r\n"), None);
        assert_eq!(parse_avd_name(""), None);
    }

    #[test]
    fn avd_label_names_image_and_device_on_both_sides() {
        assert_eq!(
            avd_label_from_config(AVD_INI),
            "android-36/google_apis/arm64-v8a on pixel"
        );
        // a requested config renders identically, so a message reads as a diff
        assert_eq!(
            avd_label("system-images/android-34/google_apis/arm64-v8a", "pixel"),
            "android-34/google_apis/arm64-v8a on pixel"
        );
    }

    #[test]
    fn avd_label_from_config_says_unknown_when_unreadable() {
        assert_eq!(
            avd_label_from_config("hw.ramSize=2048\n"),
            "unknown on unknown"
        );
    }

    #[test]
    fn hw_keyboard_flips_no_to_yes_and_leaves_siblings() {
        let ini = "hw.dPad=no\n\
                   hw.keyboard=no\n\
                   hw.keyboard.charmap=qwerty2\n\
                   hw.keyboard.lid=yes\n\
                   hw.mainKeys=no\n";
        let out = with_hw_keyboard_enabled(ini).expect("should rewrite");
        assert!(out.contains("hw.keyboard=yes"));
        // siblings untouched
        assert!(out.contains("hw.keyboard.charmap=qwerty2"));
        assert!(out.contains("hw.keyboard.lid=yes"));
        // the old value is gone and the file still ends in a newline
        assert!(!out.contains("hw.keyboard=no"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn hw_keyboard_noop_when_already_yes() {
        let ini = "hw.keyboard=yes\nhw.mainKeys=no\n";
        assert_eq!(with_hw_keyboard_enabled(ini), None);
    }

    #[test]
    fn hw_keyboard_appended_when_absent() {
        let ini = "hw.dPad=no\nhw.mainKeys=no\n";
        let out = with_hw_keyboard_enabled(ini).expect("should append");
        assert!(out.contains("hw.keyboard=yes"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn hw_keyboard_noop_does_not_match_charmap_only() {
        // a config that has the sibling keys but no bare hw.keyboard= must append.
        let ini = "hw.keyboard.charmap=qwerty2\nhw.keyboard.lid=yes\n";
        let out = with_hw_keyboard_enabled(ini).expect("should append a bare key");
        assert!(out.lines().any(|l| l == "hw.keyboard=yes"));
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
    fn classify_aab_by_extension() {
        for f in ["app.aab", "APP.AAB", "/x/My.Aab"] {
            assert_eq!(
                classify_install_input(Path::new(f), false),
                InstallInput::Aab,
                "{f} should be an aab"
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
