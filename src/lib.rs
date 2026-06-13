//! `andro` — run Android apps from the macOS command line on a native,
//! disposable Android emulator.
//!
//! Everything (a Temurin JDK, the Android SDK, and the AVDs) is provisioned
//! self-contained under `~/.andro`; nothing is installed system-wide and
//! `andro clean` wipes it. The emulator runs natively via Apple's
//! Hypervisor.framework (HVF) — no Docker, no remote host. See the README for
//! why a container cannot accelerate the emulator on Apple Silicon.

pub mod commands;
pub mod config;
pub mod emulator;
pub mod provision;
pub mod sdk;
