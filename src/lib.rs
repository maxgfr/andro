//! `andro` — run Android apps from the command line via a remote
//! docker-android (Linux+KVM) host.
//!
//! The Mac only runs this small binary plus the `docker` CLI; the Android
//! emulator lives in a disposable container on a remote Linux host that has
//! `/dev/kvm`. See the README for why local Docker on Apple Silicon cannot
//! accelerate the emulator.

pub mod commands;
pub mod config;
pub mod emulator;
pub mod provision;
pub mod sdk;
