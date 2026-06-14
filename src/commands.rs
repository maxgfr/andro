//! Command orchestration for the native emulator. Heavy lifting lives in
//! `provision`/`sdk`/`emulator`; these functions sequence the steps.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::emulator;
use crate::emulator::InstallInput;
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

/// `andro run <target>` — provision/boot, install, launch. `target` is a single
/// `.apk`, a directory of split apks, a `.xapk`/`.apks`/`.apkm` bundle, or an `.aab`
/// (converted to device-matched splits with bundletool).
pub fn run(
    cfg: &Config,
    target: &Path,
    launch: bool,
    clean_after: bool,
    downgrade: bool,
) -> Result<()> {
    if !target.exists() {
        bail!("install target not found: {}", target.display());
    }
    let s = sdk(cfg);
    provision::up(&s, cfg)?;

    let pkgs = ["shell", "pm", "list", "packages", "-3"];
    let before = emulator::parse_packages(&s.adb_capture(&pkgs).unwrap_or_default());
    let name = target.file_name().and_then(|n| n.to_str()).unwrap_or("app");
    eprintln!("📦 installing {name}…");
    install_target(&s, target, downgrade)?;
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

/// Install whatever `target` is — single apk, split-apk directory, bundle zip, or
/// `.aab` (converted to device-matched splits with bundletool first).
fn install_target(s: &Sdk, target: &Path, downgrade: bool) -> Result<()> {
    match emulator::classify_install_input(target, target.is_dir()) {
        InstallInput::Single => install_apks(s, &[target.to_path_buf()], downgrade),
        InstallInput::MultiApk => install_apks(s, &collect_apks(target)?, downgrade),
        // Unzip the bundle into scratch space, then install whatever apks it holds.
        InstallInput::Bundle => install_into_tmp(s, downgrade, |tmp| extract_zip(target, tmp)),
        // adb can't install an .aab — bundletool turns it into the splits matching
        // the booted emulator (talking to it over andro's private adb port), which
        // we then install-multiple with andro's usual flags.
        InstallInput::Aab => {
            provision::ensure_bundletool(s)?;
            install_into_tmp(s, downgrade, |tmp| build_aab_splits(s, target, tmp))
        }
    }
}

/// Populate a pid-scoped scratch dir under `~/.andro/tmp` with loose `.apk`s via
/// `produce`, install them, then sweep the dir. Pid-scoping keeps concurrent runs
/// from colliding; `autoclean`/`clean` reclaim anything left behind on a crash.
fn install_into_tmp(
    s: &Sdk,
    downgrade: bool,
    produce: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let tmp = s.tmp_dir().join(format!("extract-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    fs::create_dir_all(&tmp).with_context(|| format!("create {}", tmp.display()))?;
    let res = (|| {
        produce(&tmp)?;
        install_apks(s, &collect_apks(&tmp)?, downgrade)
    })();
    let _ = fs::remove_dir_all(&tmp);
    res
}

/// Convert an `.aab` into the splits matching the booted emulator, leaving loose
/// `.apk`s in `dest`. Three bundletool steps, all reusing one device spec so build
/// and extract agree: read the emulator's spec (abi/density/sdk/locale), build only
/// the APKs that device needs, then `extract-apks` (which reads the set's `toc.pb`)
/// emits exactly the matched splits — no mutually-exclusive standalones. The spec and
/// intermediate archive live outside `dest` so `collect_apks` only sees the apks.
fn build_aab_splits(s: &Sdk, aab: &Path, dest: &Path) -> Result<()> {
    let pid = std::process::id();
    let spec = s.tmp_dir().join(format!("aab-spec-{pid}.json"));
    let apks = s.tmp_dir().join(format!("aab-{pid}.apks"));
    let _ = fs::remove_file(&spec);
    let _ = fs::remove_file(&apks);
    let res = (|| {
        s.bundletool(&[
            "get-device-spec",
            "--connected-device",
            &format!("--adb={}", s.adb().display()),
            &format!("--output={}", spec.display()),
            "--overwrite",
        ])
        .context("bundletool get-device-spec failed (is the emulator booted?)")?;
        s.bundletool(&[
            "build-apks",
            &format!("--bundle={}", aab.display()),
            &format!("--output={}", apks.display()),
            &format!("--device-spec={}", spec.display()),
            "--overwrite",
        ])
        .context("bundletool build-apks failed")?;
        s.bundletool(&[
            "extract-apks",
            &format!("--apks={}", apks.display()),
            &format!("--device-spec={}", spec.display()),
            &format!("--output-dir={}", dest.display()),
        ])
        .context("bundletool extract-apks failed")?;
        Ok(())
    })();
    let _ = fs::remove_file(&spec);
    let _ = fs::remove_file(&apks);
    res
}

/// Install one or many apks via `adb install` / `install-multiple`, granting
/// runtime permissions (`-g`) and allowing test packages (`-t`).
fn install_apks(s: &Sdk, apks: &[PathBuf], downgrade: bool) -> Result<()> {
    if apks.is_empty() {
        bail!("no .apk files to install");
    }
    let verb = if apks.len() == 1 {
        "install"
    } else {
        "install-multiple"
    };
    let mut args: Vec<String> = vec![verb.into(), "-r".into(), "-g".into(), "-t".into()];
    if downgrade {
        args.push("-d".into());
    }
    for a in apks {
        args.push(a.to_string_lossy().into_owned());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    s.adb_capture(&refs)
        .with_context(|| format!("adb {verb} failed"))?;
    Ok(())
}

/// All `.apk` files under `dir` (recursively), sorted for a deterministic set.
fn collect_apks(dir: &Path) -> Result<Vec<PathBuf>> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
            let p = entry?.path();
            if p.is_dir() {
                walk(&p, out)?;
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
            {
                out.push(p);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, &mut out)?;
    out.sort();
    if out.is_empty() {
        bail!("no .apk files found in {}", dir.display());
    }
    Ok(out)
}

/// Extract a zip (`.xapk`/`.apks`/`.apkm`) with the system `unzip`.
fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let status = Command::new("unzip")
        .arg("-q")
        .arg("-o")
        .arg(archive)
        .arg("-d")
        .arg(dest)
        .status()
        .context("failed to spawn unzip")?;
    if !status.success() {
        bail!("unzip failed for {}", archive.display());
    }
    Ok(())
}

/// `andro launch <pkg>` — launch an already-installed app (boots if needed).
pub fn launch(cfg: &Config, pkg: &str) -> Result<()> {
    let s = sdk(cfg);
    provision::up(&s, cfg)?;
    eprintln!("🚀 launching {pkg} ({:?})…", cfg.profile);
    launch_app(&s, cfg, pkg)?;
    println!("✅ {pkg} launched");
    Ok(())
}

/// `andro uninstall <pkg>` — remove an installed app.
pub fn uninstall(cfg: &Config, pkg: &str) -> Result<()> {
    let s = sdk(cfg);
    s.adb_capture(&["uninstall", pkg])
        .with_context(|| format!("failed to uninstall {pkg}"))?;
    println!("🗑  uninstalled {pkg}");
    Ok(())
}

/// `andro shell [args…]` — passthrough to `adb shell`.
pub fn shell(cfg: &Config, args: &[String]) -> Result<()> {
    let s = sdk(cfg);
    let status = s
        .command(&s.adb())
        .arg("shell")
        .args(args)
        .status()
        .context("failed to run adb shell")?;
    if !status.success() {
        bail!("adb shell exited with an error");
    }
    Ok(())
}

/// `andro screenshot [out.png]` — capture the screen to a PNG (defaults to CWD).
pub fn screenshot(cfg: &Config, out: Option<&Path>) -> Result<()> {
    let s = sdk(cfg);
    let dest = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("screenshot.png"));
    // exec-out keeps stdout binary-clean (raw screencap -p has no tty translation).
    let png = s
        .adb_bytes(&["exec-out", "screencap", "-p"])
        .context("screencap failed (is the emulator running?)")?;
    fs::write(&dest, &png).with_context(|| format!("failed to write {}", dest.display()))?;
    println!("📸 saved {}", dest.display());
    Ok(())
}

/// `andro clear <pkg>` — wipe an app's data (`pm clear`); a fast pristine first-run.
pub fn clear(cfg: &Config, pkg: &str) -> Result<()> {
    let s = sdk(cfg);
    let out = s
        .adb_capture(&["shell", "pm", "clear", pkg])
        .with_context(|| format!("failed to clear {pkg}"))?;
    if !out.contains("Success") {
        bail!("pm clear failed for {pkg}: {out}");
    }
    println!("🧹 cleared data for {pkg}");
    Ok(())
}

/// `andro list` — installed packages (third-party by default; `--system` for all).
pub fn list(cfg: &Config, system: bool, json: bool) -> Result<()> {
    let s = sdk(cfg);
    let mut args = vec!["shell", "pm", "list", "packages"];
    if !system {
        args.push("-3");
    }
    let out = s.adb_capture(&args).context("failed to list packages")?;
    let mut pkgs = emulator::parse_packages(&out);
    pkgs.sort();
    if json {
        let body = pkgs
            .iter()
            .map(|p| format!("\"{p}\""))
            .collect::<Vec<_>>()
            .join(",");
        println!("[{body}]");
    } else {
        for p in pkgs {
            println!("{p}");
        }
    }
    Ok(())
}

/// `andro open <url>` — fire a VIEW intent (deep-link / URL-scheme testing).
pub fn open(cfg: &Config, url: &str, pkg: Option<&str>) -> Result<()> {
    let s = sdk(cfg);
    let mut args = vec![
        "shell",
        "am",
        "start",
        "-a",
        "android.intent.action.VIEW",
        "-d",
        url,
    ];
    if let Some(p) = pkg {
        args.push("-p");
        args.push(p);
    }
    // am start's exit code is unreliable; inspect the output for an Error: line.
    let (ok, out) = s.adb_output(&args);
    if !ok || out.contains("Error:") {
        bail!("could not open {url}:\n{}", out.trim());
    }
    println!("🔗 opened {url}");
    Ok(())
}

/// `andro applog <pkg>` — stream just this app's logcat lines (filtered by PID).
pub fn applog(cfg: &Config, pkg: &str, clear: bool, crash: bool) -> Result<()> {
    let s = sdk(cfg);
    let pid = s
        .adb_capture(&["shell", "pidof", pkg])
        .unwrap_or_default()
        .split_whitespace()
        .next()
        .map(str::to_string)
        .unwrap_or_default();
    if pid.is_empty() {
        bail!("{pkg} is not running — launch it first (e.g. `andro launch {pkg}`)");
    }
    if clear {
        let _ = s.adb_try(&["logcat", "-c"]);
    }
    let pid_arg = format!("--pid={pid}");
    let mut args = vec!["logcat", &pid_arg];
    if crash {
        args.push("AndroidRuntime:E");
        args.push("*:S");
    }
    let status = s
        .command(&s.adb())
        .args(&args)
        .status()
        .context("failed to run adb logcat")?;
    if !status.success() {
        bail!("adb logcat exited with an error");
    }
    Ok(())
}

/// `andro wait` — block until the running emulator is ready (scriptable barrier).
pub fn wait(cfg: &Config, timeout_secs: u64, boot_only: bool) -> Result<()> {
    let s = sdk(cfg);
    provision::wait_ready(&s, Duration::from_secs(timeout_secs), boot_only)?;
    println!("✅ ready");
    Ok(())
}

/// `andro push <local> <remote>` — copy a file onto the device.
pub fn push(cfg: &Config, local: &Path, remote: &str) -> Result<()> {
    let s = sdk(cfg);
    let l = local.to_string_lossy();
    s.adb_capture(&["push", l.as_ref(), remote])
        .with_context(|| format!("failed to push {}", local.display()))?;
    println!("⬆️  pushed {} → {remote}", local.display());
    Ok(())
}

/// `andro pull <remote> [local]` — copy a file off the device. Without `local`
/// it lands in the consolidated `~/.andro/tmp`.
pub fn pull(cfg: &Config, remote: &str, local: Option<&Path>) -> Result<()> {
    let s = sdk(cfg);
    let _ = fs::create_dir_all(s.tmp_dir());
    let dest = local.map(Path::to_path_buf).unwrap_or_else(|| {
        let name = Path::new(remote).file_name().unwrap_or_default();
        s.tmp_dir().join(name)
    });
    let d = dest.to_string_lossy();
    s.adb_capture(&["pull", remote, d.as_ref()])
        .with_context(|| format!("failed to pull {remote}"))?;
    println!("⬇️  pulled {remote} → {}", dest.display());
    Ok(())
}

/// `andro reverse <host_port> [device_port]` — bridge the Mac's port into the
/// emulator's localhost (so the device can reach a Metro/API server on the host).
pub fn reverse(
    cfg: &Config,
    host_port: Option<u16>,
    device_port: Option<u16>,
    list: bool,
    remove_all: bool,
) -> Result<()> {
    let s = sdk(cfg);
    if list {
        let out = s
            .adb_capture(&["reverse", "--list"])
            .context("adb reverse --list failed")?;
        println!("{out}");
        return Ok(());
    }
    if remove_all {
        s.adb_capture(&["reverse", "--remove-all"])
            .context("adb reverse --remove-all failed")?;
        println!("removed all reverse port bridges");
        return Ok(());
    }
    let hp = host_port.context("usage: andro reverse <host_port> [device_port]")?;
    let dp = device_port.unwrap_or(hp);
    s.adb_capture(&["reverse", &format!("tcp:{dp}"), &format!("tcp:{hp}")])
        .context("adb reverse failed")?;
    println!("🔁 device localhost:{dp} → host localhost:{hp}");
    Ok(())
}

/// `andro record [out.mp4]` — record the screen for `--time` seconds, then pull it.
/// (screenrecord hard-caps clips at 180s; no SIGINT plumbing needed.)
pub fn record(cfg: &Config, out: Option<&Path>, time_secs: u32) -> Result<()> {
    let s = sdk(cfg);
    let secs = time_secs.clamp(1, 180);
    let device_path = "/sdcard/andro-record.mp4";
    let dest = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("recording.mp4"));
    eprintln!("⏺  recording {secs}s…");
    s.adb_capture(&[
        "shell",
        "screenrecord",
        "--time-limit",
        &secs.to_string(),
        device_path,
    ])
    .context("screenrecord failed")?;
    let d = dest.to_string_lossy();
    s.adb_capture(&["pull", device_path, d.as_ref()])
        .context("failed to pull recording")?;
    let _ = s.adb_try(&["shell", "rm", device_path]);
    println!("🎬 saved {}", dest.display());
    Ok(())
}

/// `andro test <pkg>` — run instrumentation tests, exit non-zero on failure.
/// Assumes the app + test APKs are installed (e.g. via `run` on a split set).
pub fn test(cfg: &Config, pkg: &str, runner: Option<&str>, class: Option<&str>) -> Result<()> {
    let s = sdk(cfg);
    let runner = runner.unwrap_or("androidx.test.runner.AndroidJUnitRunner");
    let target = format!("{pkg}/{runner}");
    let mut args = vec!["shell", "am", "instrument", "-w"];
    if let Some(c) = class {
        args.push("-e");
        args.push("class");
        args.push(c);
    }
    args.push(&target);
    let (ok, out) = s.adb_output(&args);
    print!("{out}");
    if !ok
        || out.contains("FAILURES!!!")
        || out.contains("INSTRUMENTATION_FAILED")
        || out.contains("Error:")
    {
        bail!("instrumentation tests failed");
    }
    println!("✅ tests passed");
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

/// `andro status` — exits 0 (booted) / 2 (running, not booted) / 3 (not running)
/// so scripts can branch; `--json` prints a flat object.
pub fn status(cfg: &Config, json: bool) -> Result<()> {
    let s = sdk(cfg);
    let running = provision::is_running(&s);
    let booted = running && provision::is_booted(&s);
    if json {
        println!(
            "{{\"running\":{running},\"booted\":{booted},\"avd\":\"{}\",\"profile\":\"{:?}\",\"api\":{},\"adb_port\":{}}}",
            cfg.profile.avd_name(),
            cfg.profile,
            cfg.api,
            crate::sdk::ADB_SERVER_PORT
        );
    } else if running {
        println!("emulator: running (booted={booted})");
    } else {
        println!("emulator: not running");
    }
    // process::exit skips stdout's buffer flush — do it explicitly.
    let _ = std::io::stdout().flush();
    std::process::exit(if booted {
        0
    } else if running {
        2
    } else {
        3
    });
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
    // Check existence BEFORE any adb call: `Sdk::command` creates the contained
    // dirs, which would otherwise resurrect `~/.andro` and defeat this no-op.
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
    let s = sdk(cfg);
    // These target andro's dedicated adb port, not the user's global 5037 server.
    let _ = s.adb_try(&["emu", "kill"]);
    let _ = s.adb_try(&["kill-server"]);
    fs::remove_dir_all(&cfg.home)
        .with_context(|| format!("failed to remove {}", cfg.home.display()))?;
    println!("🗑  removed {} — nothing left behind", cfg.home.display());
    Ok(())
}

/// Options for `andro autoclean`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AutocleanOpts {
    pub dry_run: bool,
    pub reset: bool,
    pub images: bool,
    pub deep: bool,
    pub legacy: bool,
    pub yes: bool,
}

/// `andro autoclean` — reclaim temp/cache/log/stale-locks (safe), and behind
/// `--yes`, factory-reset the AVD (`--reset`) and prune unused images (`--images`).
/// Never wipes the SDK/JDK/active image (that's `clean`), never touches user files.
pub fn autoclean(cfg: &Config, opts: &AutocleanOpts) -> Result<()> {
    if !cfg.home.exists() {
        println!("nothing to clean ({} does not exist)", cfg.home.display());
        return Ok(());
    }
    let s = sdk(cfg);
    if provision::is_running(&s) {
        bail!("emulator is running — run `andro stop` first, then `andro autoclean`");
    }
    let reset = opts.reset || opts.deep;
    let images = opts.images || opts.deep;
    let dry = opts.dry_run;
    let verb = if dry { "would remove" } else { "removed" };
    let mut freed: u64 = 0;

    println!("SAFE (regenerates, no re-download):");
    freed += reclaim(&s.tmp_dir(), "tmp/", dry, verb);
    freed += reclaim(&cfg.home.join(".extract"), ".extract (legacy)", dry, verb);
    freed += reclaim(&s.android_home().join("cache"), ".android/cache", dry, verb);
    for f in [
        "emu-update-last-check.ini",
        "emu-last-feature-flags.protobuf",
        "userid",
    ] {
        freed += reclaim(
            &s.android_home().join(f),
            &format!(".android/{f}"),
            dry,
            verb,
        );
    }
    if let Ok(rd) = fs::read_dir(s.android_home()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("modem-nv-ram") {
                freed += reclaim(&e.path(), &format!(".android/{name}"), dry, verb);
            }
        }
    }
    freed += reclaim(&s.home().join("emulator.log"), "emulator.log", dry, verb);
    for avd in avd_dirs(&s) {
        let name = avd
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        for f in [
            "multiinstance.lock",
            "hardware-qemu.ini.lock",
            "tmpAdbCmds",
            "cache.img",
            "cache.img.qcow2",
        ] {
            freed += reclaim(&avd.join(f), &format!("avd/{name}/{f}"), dry, verb);
        }
    }
    println!("  {:─<54}", "");
    println!("  freed (safe): {}", emulator::human_bytes(freed));

    if reset || images {
        if !opts.yes && !dry {
            println!("\nDEEP actions need --yes (or --dry-run to preview) — nothing deep removed.");
        } else {
            println!("\nDEEP:");
            let mut deep_freed: u64 = 0;
            if reset {
                for avd in avd_dirs(&s) {
                    let name = avd
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    deep_freed += reclaim(
                        &avd.join("userdata-qemu.img.qcow2"),
                        &format!(
                            "avd/{name}/userdata-qemu.img.qcow2 (FACTORY RESET, no re-download)"
                        ),
                        dry,
                        verb,
                    );
                    deep_freed += reclaim(
                        &avd.join("snapshots"),
                        &format!("avd/{name}/snapshots"),
                        dry,
                        verb,
                    );
                }
            }
            if images {
                for img in stale_images(&s) {
                    deep_freed += reclaim(
                        &s.sdk_root().join(&img),
                        &format!("sdk/{img} (STALE IMAGE, re-download on next use)"),
                        dry,
                        verb,
                    );
                }
            }
            println!("  {:─<54}", "");
            println!("  freed (deep): {}", emulator::human_bytes(deep_freed));
            freed += deep_freed;
        }
    }

    if opts.legacy {
        freed += clean_legacy_android(dry, verb)?;
    }

    println!(
        "\ntotal {}: {}",
        if dry { "would free" } else { "freed" },
        emulator::human_bytes(freed)
    );
    list_skipped_user_files(&s);
    Ok(())
}

/// Print size, optionally delete (file or dir), return bytes accounted for.
fn reclaim(p: &Path, label: &str, dry_run: bool, verb: &str) -> u64 {
    if !p.exists() {
        return 0;
    }
    let sz = path_size(p);
    println!("  {label:<48} {:>10}   {verb}", emulator::human_bytes(sz));
    if !dry_run {
        if p.is_dir() {
            let _ = fs::remove_dir_all(p);
        } else {
            let _ = fs::remove_file(p);
        }
    }
    sz
}

/// Recursive on-disk size of a file or directory (follows no symlinks).
fn path_size(p: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(p) else {
        return 0;
    };
    if md.is_dir() {
        fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| path_size(&e.path())).sum())
            .unwrap_or(0)
    } else {
        md.len()
    }
}

/// The `<name>.avd` directories under `~/.andro/avd`.
fn avd_dirs(s: &Sdk) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(s.avd_home()) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() && p.extension().and_then(|x| x.to_str()) == Some("avd") {
                out.push(p);
            }
        }
    }
    out
}

/// Installed system images not referenced by any AVD's `config.ini` — prunable.
fn stale_images(s: &Sdk) -> Vec<String> {
    let root = s.sdk_root().join("system-images");
    let mut installed = Vec::new();
    // structure: system-images/<api>/<tag>/<abi>/
    for api in read_subdirs(&root) {
        for tag in read_subdirs(&api) {
            for abi in read_subdirs(&tag) {
                if let Ok(rel) = abi.strip_prefix(s.sdk_root()) {
                    installed.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    let referenced: Vec<String> = avd_dirs(s)
        .iter()
        .filter_map(|avd| fs::read_to_string(avd.join("config.ini")).ok())
        .filter_map(|ini| emulator::parse_image_sysdir(&ini))
        .collect();
    emulator::unreferenced_images(&installed, &referenced)
}

fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// List home-root entries that aren't andro-managed (e.g. stray screenshots) so
/// the user sees they were protected, never deleted.
fn list_skipped_user_files(s: &Sdk) {
    const ANDRO: [&str; 7] = [
        "sdk",
        "avd",
        "jdk",
        "home",
        "tmp",
        "emulator.log",
        "bundletool.jar",
    ];
    let mut skipped = Vec::new();
    if let Ok(rd) = fs::read_dir(s.home()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !ANDRO.contains(&name.as_str()) {
                skipped.push(name);
            }
        }
    }
    if !skipped.is_empty() {
        skipped.sort();
        println!("\nskipped (not andro-managed — left untouched):");
        for n in skipped {
            println!("  {n}");
        }
    }
}

/// Opt-in cleanup of the pre-containment leak in the real `~/.android` — only if
/// every entry there is andro-attributable (never touch a shared dir).
fn clean_legacy_android(dry: bool, verb: &str) -> Result<u64> {
    let Some(dir) = dirs::home_dir().map(|h| h.join(".android")) else {
        return Ok(0);
    };
    if !dir.exists() {
        return Ok(0);
    }
    let entries: Vec<String> = fs::read_dir(&dir)
        .with_context(|| format!("read {}", dir.display()))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    if !emulator::android_residue_is_andro_only(&entries) {
        println!(
            "\n⚠️  {} holds non-andro files — left untouched. Remove manually if intended.",
            dir.display()
        );
        return Ok(0);
    }
    println!("\nLEGACY ~/.android (andro-only residue):");
    let mut freed = 0;
    for name in &entries {
        freed += reclaim(&dir.join(name), &format!("~/.android/{name}"), dry, verb);
    }
    Ok(freed)
}

/// `andro logcat` — stream adb logcat. `--clear` empties the buffer first;
/// `--dump` grabs the current buffer once and exits (CI-friendly).
pub fn logcat(cfg: &Config, dump: bool, clear: bool) -> Result<()> {
    let s = sdk(cfg);
    if clear {
        let _ = s.adb_try(&["logcat", "-c"]);
    }
    if dump {
        let out = s
            .adb_capture(&["logcat", "-d"])
            .context("adb logcat -d failed")?;
        println!("{out}");
        return Ok(());
    }
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
    println!(
        "bundletool: {}",
        if s.bundletool_jar().exists() {
            "installed"
        } else {
            "missing (downloaded on first .aab)"
        }
    );
    println!(
        "adb port: {} (dedicated — andro's device isn't on the global 5037)",
        crate::sdk::ADB_SERVER_PORT
    );
    warn_legacy_residue();
    Ok(())
}

/// Note pre-containment andro residue in the real `~/.android`. Only flags the
/// actionable case (purely andro-attributable → safe to sweep); stays quiet when
/// the dir is shared with other tools, since the env fix already stops new writes.
fn warn_legacy_residue() {
    let Some(dir) = dirs::home_dir().map(|h| h.join(".android")) else {
        return;
    };
    let Ok(rd) = fs::read_dir(&dir) else {
        return;
    };
    let entries: Vec<String> = rd
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    if emulator::android_residue_is_andro_only(&entries) {
        println!(
            "⚠️  legacy andro residue in {} — `andro autoclean --legacy --yes` removes it",
            dir.display()
        );
    }
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
