//! Android SDK layer.
//!
//! Pure helpers (arch→ABI, image/URL strings, form-factor profile) are
//! unit-tested below. The `Sdk` runner holds the `~/.andro` paths and shells
//! out to the bundled JDK / sdkmanager / avdmanager / adb / emulator with the
//! right environment.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Form factor we emulate. Drives the image tag, device, AVD name and the
/// intent category used to launch an app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Phone,
    Tv,
}

impl Profile {
    /// System-image tag (`google_apis` for phones, `android-tv` for TV).
    pub fn image_tag(self) -> &'static str {
        match self {
            Profile::Phone => "google_apis",
            Profile::Tv => "android-tv",
        }
    }

    /// avdmanager device profile.
    pub fn default_device(self) -> &'static str {
        match self {
            Profile::Phone => "pixel",
            Profile::Tv => "tv_1080p",
        }
    }

    /// AVD name, kept distinct so phone and TV AVDs can coexist.
    pub fn avd_name(self) -> &'static str {
        match self {
            Profile::Phone => "andro",
            Profile::Tv => "andro-tv",
        }
    }

    /// Intent category a launcher app is started with.
    pub fn launch_category(self) -> &'static str {
        match self {
            Profile::Phone => "android.intent.category.LAUNCHER",
            Profile::Tv => "android.intent.category.LEANBACK_LAUNCHER",
        }
    }

    /// Parse a `--profile` value.
    pub fn parse(s: &str) -> Option<Profile> {
        match s.to_ascii_lowercase().as_str() {
            "phone" | "mobile" => Some(Profile::Phone),
            "tv" | "androidtv" | "android-tv" | "leanback" => Some(Profile::Tv),
            _ => None,
        }
    }
}

/// Map a Rust target arch to an Android ABI (Apple Silicon is the default).
pub fn abi_for_arch(arch: &str) -> &'static str {
    match arch {
        "x86_64" => "x86_64",
        _ => "arm64-v8a",
    }
}

/// Build a system-image package string, e.g. `system-images;android-36;android-tv;arm64-v8a`.
pub fn image_package(api: u32, abi: &str, profile: Profile) -> String {
    format!("system-images;android-{api};{};{abi}", profile.image_tag())
}

/// Adoptium Temurin 17 download URL for a Rust target arch (macOS).
pub fn jdk_url(arch: &str) -> String {
    let a = if arch == "x86_64" { "x64" } else { "aarch64" };
    format!("https://api.adoptium.net/v3/binary/latest/17/ga/mac/{a}/jdk/hotspot/normal/eclipse")
}

/// Holds the `~/.andro` layout and runs SDK tools with the right environment.
pub struct Sdk {
    home: PathBuf,
}

impl Sdk {
    pub fn new(home: PathBuf) -> Self {
        Self { home }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn sdk_root(&self) -> PathBuf {
        self.home.join("sdk")
    }
    pub fn avd_home(&self) -> PathBuf {
        self.home.join("avd")
    }
    pub fn java_home(&self) -> PathBuf {
        self.home.join("jdk/Contents/Home")
    }
    pub fn java(&self) -> PathBuf {
        self.java_home().join("bin/java")
    }
    pub fn adb(&self) -> PathBuf {
        self.sdk_root().join("platform-tools/adb")
    }
    pub fn emulator_bin(&self) -> PathBuf {
        self.sdk_root().join("emulator/emulator")
    }
    pub fn sdkmanager(&self) -> PathBuf {
        self.sdk_root().join("cmdline-tools/latest/bin/sdkmanager")
    }
    pub fn avdmanager(&self) -> PathBuf {
        self.sdk_root().join("cmdline-tools/latest/bin/avdmanager")
    }

    /// A `Command` for `program` with the SDK environment applied.
    pub fn command(&self, program: &Path) -> Command {
        let mut c = Command::new(program);
        let path = format!(
            "{}:{}:{}",
            self.sdk_root().join("platform-tools").display(),
            self.java_home().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        c.env("JAVA_HOME", self.java_home())
            .env("ANDROID_SDK_ROOT", self.sdk_root())
            .env("ANDROID_HOME", self.sdk_root())
            .env("ANDROID_AVD_HOME", self.avd_home())
            .env("PATH", path);
        c
    }

    /// Run adb with `args`, returning trimmed stdout; errors on non-zero exit.
    pub fn adb_capture(&self, args: &[&str]) -> Result<String> {
        let out = self
            .command(&self.adb())
            .args(args)
            .output()
            .context("failed to run adb")?;
        if !out.status.success() {
            bail!(
                "adb {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Run adb and return `(success, combined stdout+stderr)` without bailing —
    /// useful for tools like `monkey`/`am` whose exit code is unreliable.
    pub fn adb_output(&self, args: &[&str]) -> (bool, String) {
        match self.command(&self.adb()).args(args).output() {
            Ok(o) => {
                let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
                s.push_str(&String::from_utf8_lossy(&o.stderr));
                (o.status.success(), s)
            }
            Err(e) => (false, e.to_string()),
        }
    }

    /// Run adb tolerating failure (device may not be up yet).
    pub fn adb_try(&self, args: &[&str]) -> Option<String> {
        let out = self.command(&self.adb()).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_follows_arch() {
        assert_eq!(abi_for_arch("aarch64"), "arm64-v8a");
        assert_eq!(abi_for_arch("x86_64"), "x86_64");
        assert_eq!(abi_for_arch("anything-else"), "arm64-v8a");
    }

    #[test]
    fn image_package_phone_and_tv() {
        assert_eq!(
            image_package(36, "arm64-v8a", Profile::Phone),
            "system-images;android-36;google_apis;arm64-v8a"
        );
        assert_eq!(
            image_package(36, "arm64-v8a", Profile::Tv),
            "system-images;android-36;android-tv;arm64-v8a"
        );
    }

    #[test]
    fn jdk_url_matches_arch() {
        assert!(jdk_url("aarch64").contains("/aarch64/"));
        assert!(jdk_url("x86_64").contains("/x64/"));
        assert!(jdk_url("aarch64").starts_with("https://api.adoptium.net/"));
    }

    #[test]
    fn profile_parse_accepts_aliases() {
        assert_eq!(Profile::parse("tv"), Some(Profile::Tv));
        assert_eq!(Profile::parse("android-tv"), Some(Profile::Tv));
        assert_eq!(Profile::parse("phone"), Some(Profile::Phone));
        assert_eq!(Profile::parse("mobile"), Some(Profile::Phone));
        assert_eq!(Profile::parse("nope"), None);
    }

    #[test]
    fn tv_profile_uses_leanback_and_separate_avd() {
        assert_eq!(
            Profile::Tv.launch_category(),
            "android.intent.category.LEANBACK_LAUNCHER"
        );
        assert_eq!(Profile::Tv.avd_name(), "andro-tv");
        assert_eq!(Profile::Tv.default_device(), "tv_1080p");
        assert_eq!(Profile::Tv.image_tag(), "android-tv");
    }

    #[test]
    fn phone_profile_uses_standard_launcher() {
        assert_eq!(
            Profile::Phone.launch_category(),
            "android.intent.category.LAUNCHER"
        );
        assert_eq!(Profile::Phone.avd_name(), "andro");
        assert_eq!(Profile::Phone.default_device(), "pixel");
    }
}
