//! `andro` CLI entry point: parse args, resolve config, dispatch to a command.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{CommandFactory, Parser, Subcommand};

use andro::commands::{self, AutocleanOpts};
use andro::config::{self, BootOptions, Config, DEFAULT_API, default_home};
use andro::sdk::Profile;

#[derive(Parser)]
#[command(
    name = "andro",
    version,
    about = "Run Android apps from the macOS command line on a disposable, self-contained emulator"
)]
struct Cli {
    /// Android API level (default: latest, 36)
    #[arg(long, global = true)]
    api: Option<u32>,
    /// Use the Android TV profile (leanback launcher, TV system image)
    #[arg(long, global = true)]
    tv: bool,
    /// Form-factor profile: `phone` or `tv` (shorthand: --tv)
    #[arg(long, global = true)]
    profile: Option<String>,
    /// Use a Play Store system image (google_apis_playstore); phone only
    #[arg(long, global = true)]
    playstore: bool,
    /// Device profile override (e.g. pixel_7, tv_4k)
    #[arg(long, global = true)]
    device: Option<String>,
    /// Home dir for the self-contained SDK (default: $ANDRO_HOME or ~/.andro)
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Persist a quickboot snapshot for ~2-5s reboots (state survives until `clean`)
    #[arg(long, visible_alias = "fast", global = true)]
    snapshot: bool,
    /// Run the emulator headless (no window), e.g. on CI
    #[arg(long, global = true)]
    no_window: bool,
    /// Override emulator CPU cores
    #[arg(long, global = true)]
    cores: Option<u32>,
    /// Override emulator RAM in MB
    #[arg(long, global = true)]
    memory: Option<u32>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Provision (first run), boot, install and launch. Target is a .apk, a
    /// directory of split apks, a .xapk/.apks/.apkm bundle, or an .aab.
    Run {
        /// Path to a .apk, a split-apk directory, a bundle archive, or an .aab
        apk: PathBuf,
        /// Install without launching
        #[arg(long)]
        no_launch: bool,
        /// Tear everything down (clean) after launching
        #[arg(long)]
        clean_after: bool,
        /// Allow installing an older version over a newer one (adb -d)
        #[arg(long)]
        downgrade: bool,
    },
    /// Install an app without launching it (same inputs as `run`)
    Install {
        /// Path to a .apk, a split-apk directory, a bundle archive, or an .aab
        apk: PathBuf,
        /// Allow installing an older version over a newer one (adb -d)
        #[arg(long)]
        downgrade: bool,
    },
    /// Launch an already-installed package
    Launch {
        /// Application package id (e.g. com.example.app)
        pkg: String,
    },
    /// Uninstall a package
    Uninstall {
        /// Application package id (e.g. com.example.app)
        pkg: String,
    },
    /// Wipe an app's data (`pm clear`) for a fast pristine first-run
    Clear {
        /// Application package id
        pkg: String,
    },
    /// List installed packages (third-party by default)
    List {
        /// Include system packages too
        #[arg(long)]
        system: bool,
        /// Emit a JSON array
        #[arg(long)]
        json: bool,
    },
    /// Open a URL / deep link with a VIEW intent
    Open {
        /// URL or custom-scheme link (e.g. https://… or myapp://…)
        url: String,
        /// Restrict the intent to this package
        #[arg(long)]
        pkg: Option<String>,
    },
    /// Capture a screenshot to a PNG (defaults to ./screenshot.png)
    Screenshot {
        /// Output path
        out: Option<PathBuf>,
    },
    /// Record the screen for a few seconds, then save an MP4
    Record {
        /// Output path (defaults to ./recording.mp4)
        out: Option<PathBuf>,
        /// Seconds to record (max 180)
        #[arg(long, default_value_t = 30)]
        time: u32,
    },
    /// Run `adb shell` with the given args (e.g. `andro shell getprop`)
    Shell {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Stream just one app's logcat lines (filtered by PID)
    Applog {
        /// Application package id (must be running)
        pkg: String,
        /// Empty the log buffer first
        #[arg(long)]
        clear: bool,
        /// Show only the crash stack (AndroidRuntime)
        #[arg(long)]
        crash: bool,
    },
    /// Copy a file onto the device
    Push { local: PathBuf, remote: String },
    /// Copy a file off the device (defaults into ~/.andro/tmp)
    Pull {
        remote: String,
        local: Option<PathBuf>,
    },
    /// Bridge a Mac host port into the emulator's localhost (adb reverse)
    Reverse {
        /// Host port to expose (also the device port unless one is given)
        host_port: Option<u16>,
        /// Device-side port (defaults to host_port)
        device_port: Option<u16>,
        /// List active bridges
        #[arg(long)]
        list: bool,
        /// Remove all bridges
        #[arg(long)]
        remove_all: bool,
    },
    /// Run instrumentation tests (am instrument), exit non-zero on failure
    Test {
        /// App-under-test / test package id
        pkg: String,
        /// Instrumentation runner (default AndroidJUnitRunner)
        #[arg(long)]
        runner: Option<String>,
        /// Restrict to a single test class
        #[arg(long)]
        class: Option<String>,
    },
    /// Provision and boot the emulator
    Up,
    /// Block until the running emulator is ready (scriptable barrier)
    Wait {
        /// Timeout in seconds (default 300)
        #[arg(long)]
        timeout: Option<u64>,
        /// Stop at sys.boot_completed (skip the PackageManager-ready wait)
        #[arg(long = "boot-only")]
        boot_only: bool,
    },
    /// Show emulator status (exit 0 booted / 2 running / 3 not running)
    Status {
        /// Emit a JSON object
        #[arg(long)]
        json: bool,
    },
    /// Stop the emulator (keep ~/.andro)
    Stop,
    /// Kill the emulator and remove ~/.andro
    Clean {
        /// Confirm the destructive removal
        #[arg(long)]
        yes: bool,
    },
    /// Reclaim temp/cache/logs (safe); --reset/--images/--deep need --yes
    Autoclean {
        /// Show what would be freed, delete nothing
        #[arg(long)]
        dry_run: bool,
        /// Also factory-reset the AVD (wipe userdata; no re-download)
        #[arg(long)]
        reset: bool,
        /// Also prune system images no AVD references (re-download on next use)
        #[arg(long)]
        images: bool,
        /// = --reset + --images
        #[arg(long)]
        deep: bool,
        /// Also remove andro-only residue from the legacy ~/.android
        #[arg(long)]
        legacy: bool,
        /// Confirm the deep/legacy removals
        #[arg(long)]
        yes: bool,
    },
    /// Stream adb logcat from the emulator
    Logcat {
        /// Grab the current buffer once and exit (CI-friendly)
        #[arg(long)]
        dump: bool,
        /// Empty the log buffer first
        #[arg(long)]
        clear: bool,
    },
    /// Check macOS/Apple Silicon, Hypervisor.framework, and resolved paths
    Doctor,
    /// Print a shell completion script (bash, zsh, fish, …)
    Completions { shell: clap_complete::Shell },
}

fn build_config(cli: &Cli) -> Result<Config> {
    let home = cli.home.clone().unwrap_or_else(default_home);
    let file = config::load_config(&home);
    // Precedence everywhere: CLI flag > ANDRO_* env > config file > built-in.
    let api = cli
        .api
        .or_else(|| std::env::var("ANDRO_API").ok().and_then(|v| v.parse().ok()))
        .or(file.api)
        .unwrap_or(DEFAULT_API);
    let profile = if let Some(p) = cli.profile.as_deref() {
        Profile::parse(p)
            .ok_or_else(|| anyhow!("unknown --profile '{p}' (expected: phone or tv)"))?
    } else if cli.tv {
        Profile::Tv
    } else if let Some(p) = file.profile.as_deref() {
        Profile::parse(p)
            .ok_or_else(|| anyhow!("unknown profile '{p}' in config (expected: phone or tv)"))?
    } else {
        Profile::Phone
    };
    let playstore = cli.playstore || file.playstore.unwrap_or(false);
    if playstore && profile == Profile::Tv {
        bail!("--playstore is not available for the TV profile (no Play Store TV image)");
    }
    let boot = BootOptions {
        snapshot: cli.snapshot || file.snapshot.unwrap_or(false),
        no_window: cli.no_window || file.no_window.unwrap_or(false),
        cores: cli.cores.or(file.cores),
        memory: cli.memory.or(file.memory),
    };
    Ok(Config::build(
        home,
        api,
        profile,
        playstore,
        cli.device.clone().or(file.device),
        std::env::consts::ARCH,
        boot,
    ))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    // Completions need no config or emulator — handle before anything else.
    if let Command::Completions { shell } = &cli.command {
        clap_complete::generate(*shell, &mut Cli::command(), "andro", &mut std::io::stdout());
        return Ok(());
    }
    let cfg = build_config(&cli)?;
    match &cli.command {
        Command::Run {
            apk,
            no_launch,
            clean_after,
            downgrade,
        } => commands::run(&cfg, apk, !no_launch, *clean_after, *downgrade),
        Command::Install { apk, downgrade } => commands::run(&cfg, apk, false, false, *downgrade),
        Command::Launch { pkg } => commands::launch(&cfg, pkg),
        Command::Uninstall { pkg } => commands::uninstall(&cfg, pkg),
        Command::Clear { pkg } => commands::clear(&cfg, pkg),
        Command::List { system, json } => commands::list(&cfg, *system, *json),
        Command::Open { url, pkg } => commands::open(&cfg, url, pkg.as_deref()),
        Command::Screenshot { out } => commands::screenshot(&cfg, out.as_deref()),
        Command::Record { out, time } => commands::record(&cfg, out.as_deref(), *time),
        Command::Shell { args } => commands::shell(&cfg, args),
        Command::Applog { pkg, clear, crash } => commands::applog(&cfg, pkg, *clear, *crash),
        Command::Push { local, remote } => commands::push(&cfg, local, remote),
        Command::Pull { remote, local } => commands::pull(&cfg, remote, local.as_deref()),
        Command::Reverse {
            host_port,
            device_port,
            list,
            remove_all,
        } => commands::reverse(&cfg, *host_port, *device_port, *list, *remove_all),
        Command::Test { pkg, runner, class } => {
            commands::test(&cfg, pkg, runner.as_deref(), class.as_deref())
        }
        Command::Up => commands::up(&cfg),
        Command::Wait { timeout, boot_only } => {
            commands::wait(&cfg, timeout.unwrap_or(300), *boot_only)
        }
        Command::Status { json } => commands::status(&cfg, *json),
        Command::Stop => commands::stop(&cfg),
        Command::Clean { yes } => commands::clean(&cfg, *yes),
        Command::Autoclean {
            dry_run,
            reset,
            images,
            deep,
            legacy,
            yes,
        } => commands::autoclean(
            &cfg,
            &AutocleanOpts {
                dry_run: *dry_run,
                reset: *reset,
                images: *images,
                deep: *deep,
                legacy: *legacy,
                yes: *yes,
            },
        ),
        Command::Logcat { dump, clear } => commands::logcat(&cfg, *dump, *clear),
        Command::Doctor => commands::doctor(&cfg),
        Command::Completions { .. } => unreachable!("handled above"),
    }
}
