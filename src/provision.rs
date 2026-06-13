//! Self-contained provisioning of `~/.andro`: download a JDK, the Android SDK
//! (command-line tools + platform-tools + emulator + a system image), create an
//! AVD, and boot the emulator. No brew, nothing installed system-wide.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::emulator;
use crate::sdk::{self, Sdk};

const CMDLINE_TOOLS_URL_DEFAULT: &str =
    "https://dl.google.com/android/repository/commandlinetools-mac-11076708_latest.zip";
const BOOT_TIMEOUT: Duration = Duration::from_secs(300);
const POLL: Duration = Duration::from_secs(3);

fn run_checked(mut cmd: Command, what: &str) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("failed to spawn: {what}"))?;
    if !status.success() {
        bail!("{what} failed (exit {:?})", status.code());
    }
    Ok(())
}

fn download(url: &str, dest: &Path) -> Result<()> {
    let mut c = Command::new("curl");
    c.args(["-fL", "--retry", "3", "-o"]).arg(dest).arg(url);
    run_checked(c, &format!("download {url}"))
}

/// Run the whole chain: JDK → SDK → AVD → boot.
pub fn up(sdk: &Sdk, cfg: &Config) -> Result<()> {
    ensure_jdk(sdk, std::env::consts::ARCH)?;
    ensure_sdk(sdk, &cfg.image())?;
    ensure_avd(sdk, cfg)?;
    boot(sdk, cfg)
}

pub fn ensure_jdk(sdk: &Sdk, arch: &str) -> Result<()> {
    if sdk.java().exists() {
        return Ok(());
    }
    fs::create_dir_all(sdk.home())?;
    let tar = sdk.home().join("jdk.tar.gz");
    eprintln!("⬇️  downloading JDK 17…");
    download(&sdk::jdk_url(arch), &tar)?;
    let jdk_dir = sdk.home().join("jdk");
    fs::create_dir_all(&jdk_dir)?;
    let mut c = Command::new("tar");
    c.arg("-xzf")
        .arg(&tar)
        .arg("-C")
        .arg(&jdk_dir)
        .args(["--strip-components", "1"]);
    run_checked(c, "extract JDK")?;
    let _ = fs::remove_file(&tar);
    Ok(())
}

pub fn ensure_sdk(sdk: &Sdk, image: &str) -> Result<()> {
    let image_dir = sdk.sdk_root().join(image.replace(';', "/"));
    if sdk.adb().exists() && sdk.emulator_bin().exists() && image_dir.exists() {
        return Ok(());
    }
    if !sdk.sdkmanager().exists() {
        eprintln!("⬇️  downloading Android command-line tools…");
        let zip = sdk.home().join("cmdline-tools.zip");
        let url = std::env::var("ANDRO_CMDLINE_TOOLS_URL")
            .unwrap_or_else(|_| CMDLINE_TOOLS_URL_DEFAULT.to_string());
        download(&url, &zip)?;
        let dest = sdk.sdk_root().join("cmdline-tools");
        fs::create_dir_all(&dest)?;
        let mut c = Command::new("unzip");
        c.arg("-q").arg("-o").arg(&zip).arg("-d").arg(&dest);
        run_checked(c, "unzip cmdline-tools")?;
        // the zip extracts to cmdline-tools/cmdline-tools/* — move it to /latest
        let extracted = dest.join("cmdline-tools");
        let latest = dest.join("latest");
        if extracted.exists() {
            let _ = fs::remove_dir_all(&latest);
            fs::rename(&extracted, &latest).context("move cmdline-tools to latest")?;
        }
        let _ = fs::remove_file(&zip);
    }
    accept_licenses(sdk);
    eprintln!("⬇️  installing platform-tools, emulator and system image (can take a while)…");
    let mut c = sdk.command(&sdk.sdkmanager());
    c.arg(format!("--sdk_root={}", sdk.sdk_root().display()))
        .arg("platform-tools")
        .arg("emulator")
        .arg(image);
    run_checked(c, "sdkmanager install")
}

fn accept_licenses(sdk: &Sdk) {
    let child = sdk
        .command(&sdk.sdkmanager())
        .arg(format!("--sdk_root={}", sdk.sdk_root().display()))
        .arg("--licenses")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all("y\n".repeat(50).as_bytes());
        }
        let _ = child.wait();
    }
}

pub fn ensure_avd(sdk: &Sdk, cfg: &Config) -> Result<()> {
    let avd_ini = sdk
        .avd_home()
        .join(format!("{}.ini", cfg.profile.avd_name()));
    if avd_ini.exists() {
        return Ok(());
    }
    fs::create_dir_all(sdk.avd_home())?;
    eprintln!("🛠  creating AVD '{}'…", cfg.profile.avd_name());
    let mut child = sdk
        .command(&sdk.avdmanager())
        .args([
            "create",
            "avd",
            "-n",
            cfg.profile.avd_name(),
            "-k",
            &cfg.image(),
            "--device",
            &cfg.device,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn avdmanager")?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"no\n");
    }
    let out = child.wait_with_output().context("avdmanager create")?;
    if !out.status.success() {
        bail!(
            "avdmanager create failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// True if any emulator instance is attached to adb.
pub fn is_running(sdk: &Sdk) -> bool {
    sdk.adb_try(&["devices"])
        .map(|o| {
            o.lines()
                .any(|l| l.contains("emulator-") && l.contains("device"))
        })
        .unwrap_or(false)
}

/// True if the booted property is set.
pub fn is_booted(sdk: &Sdk) -> bool {
    sdk.adb_try(&["shell", "getprop", "sys.boot_completed"])
        .map(|o| emulator::boot_completed(&o))
        .unwrap_or(false)
}

pub fn boot(sdk: &Sdk, cfg: &Config) -> Result<()> {
    if is_booted(sdk) {
        return Ok(());
    }
    if !is_running(sdk) {
        eprintln!("🚀 booting emulator ({})…", cfg.profile.avd_name());
        let log = sdk.home().join("emulator.log");
        let mut c = sdk.command(&sdk.emulator_bin());
        c.args([
            "-avd",
            cfg.profile.avd_name(),
            "-no-snapshot",
            "-no-boot-anim",
            "-gpu",
            "auto",
        ])
        .stdin(Stdio::null());
        if let Ok(f) = fs::File::create(&log)
            && let Ok(f2) = f.try_clone()
        {
            c.stdout(Stdio::from(f)).stderr(Stdio::from(f2));
        }
        c.spawn().context("failed to launch emulator")?; // detached
    }
    wait_for_boot(sdk)
}

fn wait_for_boot(sdk: &Sdk) -> Result<()> {
    let start = Instant::now();
    let _ = sdk.adb_try(&["wait-for-device"]);
    loop {
        if is_booted(sdk) {
            eprintln!("✅ emulator booted");
            return Ok(());
        }
        if start.elapsed() > BOOT_TIMEOUT {
            bail!("emulator did not boot within {}s", BOOT_TIMEOUT.as_secs());
        }
        sleep(POLL);
    }
}
