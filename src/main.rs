//! `andro` CLI entry point: parse args, resolve config, dispatch to a command.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use andro::commands;
use andro::config::{Config, DEFAULT_API, default_home};
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
    /// Device profile override (e.g. pixel_7, tv_4k)
    #[arg(long, global = true)]
    device: Option<String>,
    /// Home dir for the self-contained SDK (default: $ANDRO_HOME or ~/.andro)
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Provision (first run), boot, install the APK and launch it
    Run {
        /// Path to the .apk file
        apk: PathBuf,
        /// Install without launching
        #[arg(long)]
        no_launch: bool,
        /// Tear everything down (clean) after launching
        #[arg(long)]
        clean_after: bool,
    },
    /// Provision and boot the emulator
    Up,
    /// Show emulator status
    Status,
    /// Stop the emulator (keep ~/.andro)
    Stop,
    /// Kill the emulator and remove ~/.andro
    Clean {
        /// Confirm the destructive removal
        #[arg(long)]
        yes: bool,
    },
    /// Stream adb logcat from the emulator
    Logcat,
    /// Check macOS/Apple Silicon, Hypervisor.framework, and resolved paths
    Doctor,
}

fn build_config(cli: &Cli) -> Config {
    let home = cli.home.clone().unwrap_or_else(default_home);
    let api = cli
        .api
        .or_else(|| std::env::var("ANDRO_API").ok().and_then(|v| v.parse().ok()))
        .unwrap_or(DEFAULT_API);
    let profile = if cli.tv { Profile::Tv } else { Profile::Phone };
    Config::build(
        home,
        api,
        profile,
        cli.device.clone(),
        std::env::consts::ARCH,
    )
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = build_config(&cli);
    match &cli.command {
        Command::Run {
            apk,
            no_launch,
            clean_after,
        } => commands::run(&cfg, apk, !no_launch, *clean_after),
        Command::Up => commands::up(&cfg),
        Command::Status => commands::status(&cfg),
        Command::Stop => commands::stop(&cfg),
        Command::Clean { yes } => commands::clean(&cfg, *yes),
        Command::Logcat => commands::logcat(&cfg),
        Command::Doctor => commands::doctor(&cfg),
    }
}
