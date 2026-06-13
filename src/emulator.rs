//! Pure helpers for talking to the emulator over adb (inside the container).
//!
//! These parse the textual output of `adb` commands. The orchestration that
//! actually runs them lives in `commands/`; keeping the parsing pure makes it
//! unit-testable without a running emulator.

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

/// Extract the `pkg/activity` component from `cmd package resolve-activity
/// --brief -c <category> <pkg>` output. `None` if nothing resolved. (RED stub.)
pub fn parse_resolved_activity(output: &str, pkg: &str) -> Option<String> {
    let prefix = format!("{pkg}/");
    output
        .lines()
        .map(|l| l.trim())
        .find(|l| l.starts_with(&prefix))
        .map(|l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // adb over the container often yields CRLF; non-package lines are dropped.
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
