//! Command orchestration for the native emulator. Heavy lifting lives in
//! `provision`/`sdk`/`emulator`; these functions sequence the steps.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::emulator;
use crate::provision;
use crate::sdk::{Profile, Sdk};

fn sdk(cfg: &Config) -> Sdk {
    Sdk::new(cfg.home.clone())
}

/// `andro up` — provision and boot.
pub fn up(cfg: &Config) -> Result<()> {
    let s = sdk(cfg);
    provision::up(&s, cfg)?;
    println!(
        "✅ emulator ready ({:?}, android-{}, {})",
        cfg.profile, cfg.api, cfg.device
    );
    Ok(())
}

/// `andro run <apk>` — provision/boot, install the APK, launch it.
pub fn run(cfg: &Config, apk: &Path, launch: bool, clean_after: bool) -> Result<()> {
    if !apk.exists() {
        bail!("APK not found: {}", apk.display());
    }
    let s = sdk(cfg);
    provision::up(&s, cfg)?;

    let pkgs = ["shell", "pm", "list", "packages", "-3"];
    let before = emulator::parse_packages(&s.adb_capture(&pkgs).unwrap_or_default());
    let name = apk
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app.apk");
    eprintln!("📦 installing {name}…");
    s.adb_capture(&["install", "-r", &apk.to_string_lossy()])
        .context("adb install failed")?;
    let after = emulator::parse_packages(&s.adb_capture(&pkgs)?);

    let pkg = emulator::newly_installed(&before, &after)
        .into_iter()
        .next()
        .or_else(|| after.last().cloned())
        .context("could not determine the installed package")?;
    println!("✅ installed {pkg}");

    if launch {
        eprintln!("🚀 launching {pkg} ({:?})…", cfg.profile);
        launch_app(&s, cfg, &pkg)?;
        println!("✅ {pkg} launched");
    }
    if clean_after {
        clean(cfg, true)?;
    }
    Ok(())
}

/// Launch the app by resolving its launchable activity for the profile's
/// category (falling back to the other category), then `am start`. This avoids
/// `monkey`, whose exit code is unreliable.
fn launch_app(s: &Sdk, cfg: &Config, pkg: &str) -> Result<()> {
    let categories = match cfg.profile {
        Profile::Tv => [Profile::Tv, Profile::Phone],
        Profile::Phone => [Profile::Phone, Profile::Tv],
    };
    for profile in categories {
        let cat = profile.launch_category();
        let (_, resolved) = s.adb_output(&[
            "shell",
            "cmd",
            "package",
            "resolve-activity",
            "--brief",
            "-c",
            cat,
            pkg,
        ]);
        if let Some(component) = emulator::parse_resolved_activity(&resolved, pkg) {
            let (ok, out) = s.adb_output(&["shell", "am", "start", "-n", &component]);
            if ok && !out.contains("Error:") {
                return Ok(());
            }
        }
    }
    bail!("could not find a launchable activity for {pkg}");
}

/// `andro status`.
pub fn status(cfg: &Config) -> Result<()> {
    let s = sdk(cfg);
    if provision::is_running(&s) {
        println!("emulator: running (booted={})", provision::is_booted(&s));
    } else {
        println!("emulator: not running");
    }
    Ok(())
}

/// `andro stop` — stop the emulator, keep `~/.andro`.
pub fn stop(cfg: &Config) -> Result<()> {
    let s = sdk(cfg);
    if provision::is_running(&s) {
        let _ = s.adb_try(&["emu", "kill"]);
        println!("⏹  emulator stopped");
    } else {
        println!("emulator not running");
    }
    Ok(())
}

/// `andro clean` — kill the emulator and remove `~/.andro`.
pub fn clean(cfg: &Config, yes: bool) -> Result<()> {
    let s = sdk(cfg);
    let _ = s.adb_try(&["emu", "kill"]);
    let _ = s.adb_try(&["kill-server"]);
    if !cfg.home.exists() {
        println!("nothing to clean ({} does not exist)", cfg.home.display());
        return Ok(());
    }
    if !yes {
        println!(
            "This will permanently remove {}.\nRe-run with --yes to confirm.",
            cfg.home.display()
        );
        return Ok(());
    }
    fs::remove_dir_all(&cfg.home)
        .with_context(|| format!("failed to remove {}", cfg.home.display()))?;
    println!("🗑  removed {} — nothing left behind", cfg.home.display());
    Ok(())
}

/// `andro logcat` — stream adb logcat.
pub fn logcat(cfg: &Config) -> Result<()> {
    let s = sdk(cfg);
    let status = s
        .command(&s.adb())
        .arg("logcat")
        .status()
        .context("failed to run adb logcat")?;
    if !status.success() {
        bail!("adb logcat exited with an error");
    }
    Ok(())
}

/// `andro doctor` — environment + resolved config.
pub fn doctor(cfg: &Config) -> Result<()> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    println!("os/arch: {os} / {arch}");
    if os != "macos" {
        println!("⚠️  andro's native emulator needs macOS (Hypervisor.framework).");
    } else if arch == "aarch64" {
        println!("✅ Apple Silicon — arm64 images run natively via HVF");
    } else {
        println!("ℹ️  Intel Mac — will use x86_64 images");
    }
    if let Some(hv) = hv_support() {
        println!(
            "{} Hypervisor.framework: {}",
            if hv { "✅" } else { "❌" },
            if hv { "available" } else { "unavailable" }
        );
    }
    println!("home:    {}", cfg.home.display());
    println!(
        "api:     {}   profile: {:?}   device: {}",
        cfg.api, cfg.profile, cfg.device
    );
    println!("image:   {}", cfg.image());
    let s = sdk(cfg);
    println!(
        "jdk:     {}",
        if s.java().exists() {
            "installed"
        } else {
            "missing (downloaded on first run)"
        }
    );
    println!(
        "sdk:     {}",
        if s.sdkmanager().exists() {
            "installed"
        } else {
            "missing (downloaded on first run)"
        }
    );
    Ok(())
}

/// `sysctl kern.hv_support` → Some(true/false) on macOS, None elsewhere.
fn hv_support() -> Option<bool> {
    if std::env::consts::OS != "macos" {
        return None;
    }
    let out = std::process::Command::new("sysctl")
        .args(["-n", "kern.hv_support"])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim() == "1")
}
