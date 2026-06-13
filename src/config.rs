//! Resolved configuration for the native emulator.
//!
//! Effective values come from CLI flags, then `ANDRO_*` env, then defaults.
//! The home directory (`~/.andro`) holds the self-contained JDK + SDK + AVDs.

use std::path::{Path, PathBuf};

use crate::sdk::{self, Profile};

/// Default Android API level (latest available system image).
pub const DEFAULT_API: u32 = 36;

/// Emulator boot tuning. All opt-in; the defaults preserve a disposable cold
/// boot so each `up`/`run` starts from a clean device.
#[derive(Debug, Clone, Default)]
pub struct BootOptions {
    /// Persist/reuse the quickboot snapshot for ~2-5s reboots. Off by default:
    /// a saved snapshot keeps installed apps/state until `andro clean`.
    pub snapshot: bool,
    /// Run the emulator headless (`-no-window`), e.g. on CI.
    pub no_window: bool,
    /// Override the number of emulator CPU cores (else the emulator auto-sizes).
    pub cores: Option<u32>,
    /// Override the emulator RAM in MB (else the emulator auto-sizes).
    pub memory: Option<u32>,
}

/// Fully-resolved configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub api: u32,
    pub profile: Profile,
    /// Use the Play Store system image (`google_apis_playstore`) — phone only.
    pub playstore: bool,
    /// Resolved avdmanager device profile (override, else the profile default).
    pub device: String,
    /// Android ABI derived from the host arch.
    pub abi: String,
    /// Emulator boot tuning.
    pub boot: BootOptions,
}

impl Config {
    /// Build a config, filling `device` from the profile when not overridden
    /// and deriving the ABI from `arch`.
    pub fn build(
        home: PathBuf,
        api: u32,
        profile: Profile,
        playstore: bool,
        device: Option<String>,
        arch: &str,
        boot: BootOptions,
    ) -> Config {
        Config {
            home,
            api,
            profile,
            playstore,
            device: device.unwrap_or_else(|| profile.default_device().to_string()),
            abi: sdk::abi_for_arch(arch).to_string(),
            boot,
        }
    }

    /// System-image package string for this config.
    pub fn image(&self) -> String {
        sdk::image_package(self.api, &self.abi, self.profile, self.playstore)
    }
}

/// Optional defaults read from `<home>/config.toml`. Lowest precedence:
/// CLI flag > `ANDRO_*` env > config file > built-in default.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FileDefaults {
    pub api: Option<u32>,
    pub profile: Option<String>,
    pub device: Option<String>,
    pub playstore: Option<bool>,
    pub snapshot: Option<bool>,
    pub no_window: Option<bool>,
    pub cores: Option<u32>,
    pub memory: Option<u32>,
}

/// Parse a simple `key = value` config (with `#` comments and optional quotes).
pub fn parse_config(text: &str) -> FileDefaults {
    let mut d = FileDefaults::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim();
        match k.trim() {
            "api" => d.api = v.parse().ok(),
            "profile" => d.profile = Some(v.to_string()),
            "device" => d.device = Some(v.to_string()),
            "playstore" => d.playstore = parse_bool(v),
            "snapshot" | "fast" => d.snapshot = parse_bool(v),
            "no_window" | "no-window" => d.no_window = parse_bool(v),
            "cores" => d.cores = v.parse().ok(),
            "memory" => d.memory = v.parse().ok(),
            _ => {}
        }
    }
    d
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Load `<home>/config.toml` if present; missing/invalid file → all defaults.
pub fn load_config(home: &Path) -> FileDefaults {
    std::fs::read_to_string(home.join("config.toml"))
        .map(|t| parse_config(&t))
        .unwrap_or_default()
}

/// Default home: `$ANDRO_HOME` or `~/.andro`.
pub fn default_home() -> PathBuf {
    if let Ok(h) = std::env::var("ANDRO_HOME")
        && !h.is_empty()
    {
        return PathBuf::from(h);
    }
    dirs::home_dir().unwrap_or_default().join(".andro")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn device_falls_back_to_profile_default() {
        let c = Config::build(
            PathBuf::from("/h"),
            36,
            Profile::Tv,
            false,
            None,
            "aarch64",
            BootOptions::default(),
        );
        assert_eq!(c.device, "tv_1080p");
    }

    #[test]
    fn device_override_wins() {
        let c = Config::build(
            PathBuf::from("/h"),
            36,
            Profile::Phone,
            false,
            Some("pixel_7".to_string()),
            "aarch64",
            BootOptions::default(),
        );
        assert_eq!(c.device, "pixel_7");
    }

    #[test]
    fn abi_and_image_track_arch_and_profile() {
        let c = Config::build(
            Path::new("/h").to_path_buf(),
            34,
            Profile::Tv,
            false,
            None,
            "aarch64",
            BootOptions::default(),
        );
        assert_eq!(c.abi, "arm64-v8a");
        assert_eq!(c.image(), "system-images;android-34;android-tv;arm64-v8a");
    }

    #[test]
    fn parse_config_reads_keys_comments_and_quotes() {
        let text = "\
            # andro defaults\n\
            api = 34\n\
            profile = \"tv\"\n\
            device=pixel_7\n\
            playstore = true\n\
            snapshot = yes\n\
            no_window = off\n\
            cores = 6\n\
            memory = 4096\n\
            \n\
            bogus line with no equals\n";
        let d = parse_config(text);
        assert_eq!(d.api, Some(34));
        assert_eq!(d.profile.as_deref(), Some("tv"));
        assert_eq!(d.device.as_deref(), Some("pixel_7"));
        assert_eq!(d.playstore, Some(true));
        assert_eq!(d.snapshot, Some(true));
        assert_eq!(d.no_window, Some(false));
        assert_eq!(d.cores, Some(6));
        assert_eq!(d.memory, Some(4096));
    }

    #[test]
    fn parse_config_empty_is_all_none() {
        assert_eq!(parse_config(""), FileDefaults::default());
        assert_eq!(parse_config("# only a comment\n"), FileDefaults::default());
    }

    #[test]
    fn playstore_selects_playstore_image() {
        let c = Config::build(
            PathBuf::from("/h"),
            36,
            Profile::Phone,
            true,
            None,
            "aarch64",
            BootOptions::default(),
        );
        assert_eq!(
            c.image(),
            "system-images;android-36;google_apis_playstore;arm64-v8a"
        );
    }
}
