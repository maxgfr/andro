//! Resolved configuration for the native emulator.
//!
//! Effective values come from CLI flags, then `ANDRO_*` env, then defaults.
//! The home directory (`~/.andro`) holds the self-contained JDK + SDK + AVDs.

use std::path::PathBuf;

use crate::sdk::{self, Profile};

/// Default Android API level (latest available system image).
pub const DEFAULT_API: u32 = 36;

/// Fully-resolved configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub api: u32,
    pub profile: Profile,
    /// Resolved avdmanager device profile (override, else the profile default).
    pub device: String,
    /// Android ABI derived from the host arch.
    pub abi: String,
}

impl Config {
    /// Build a config, filling `device` from the profile when not overridden
    /// and deriving the ABI from `arch`.
    pub fn build(
        home: PathBuf,
        api: u32,
        profile: Profile,
        device: Option<String>,
        arch: &str,
    ) -> Config {
        Config {
            home,
            api,
            profile,
            device: device.unwrap_or_else(|| profile.default_device().to_string()),
            abi: sdk::abi_for_arch(arch).to_string(),
        }
    }

    /// System-image package string for this config.
    pub fn image(&self) -> String {
        sdk::image_package(self.api, &self.abi, self.profile)
    }
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
        let c = Config::build(PathBuf::from("/h"), 36, Profile::Tv, None, "aarch64");
        assert_eq!(c.device, "tv_1080p");
    }

    #[test]
    fn device_override_wins() {
        let c = Config::build(
            PathBuf::from("/h"),
            36,
            Profile::Phone,
            Some("pixel_7".to_string()),
            "aarch64",
        );
        assert_eq!(c.device, "pixel_7");
    }

    #[test]
    fn abi_and_image_track_arch_and_profile() {
        let c = Config::build(
            Path::new("/h").to_path_buf(),
            34,
            Profile::Tv,
            None,
            "aarch64",
        );
        assert_eq!(c.abi, "arm64-v8a");
        assert_eq!(c.image(), "system-images;android-34;android-tv;arm64-v8a");
    }
}
