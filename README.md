# andro

Run Android apps — phone **or smart TV** — from your macOS command line, on a
fast, **disposable** Android emulator. No Android Studio, no brew, nothing
installed system-wide: everything lives in `~/.andro` and `andro clean` wipes it.

```sh
andro run app.apk          # boot a phone emulator, install, launch
andro --tv run tvapp.apk   # same, on an Android TV (leanback) emulator
andro clean --yes          # kill the emulator and delete ~/.andro
```

The emulator runs **natively on Apple Silicon** via Apple's Hypervisor.framework
(HVF) with **arm64** system images, so it boots in seconds and runs arm64 apps
without translation.

> Why native and not Docker? The Android emulator needs hardware virtualization.
> On a Mac that's HVF, which only a native process can use — a container can't
> (`docker-android` needs Linux + `/dev/kvm`). So `andro` drives the real SDK
> emulator directly.

## Requirements

- **macOS** (Apple Silicon recommended; Intel works with x86_64 images).
- That's it — `andro` downloads a self-contained JDK + Android SDK into
  `~/.andro` on first run (~2 GB).

## Install

```sh
cargo install --path .        # or grab a binary from the GitHub Releases
```

## Usage

```sh
andro run <app.apk>        # provision (first run) → boot → install → launch
andro --tv run <app.apk>   # Android TV profile (TV image + leanback launcher)
andro up                   # just provision + boot
andro status               # is the emulator running / booted?
andro stop                 # stop the emulator, keep ~/.andro
andro clean --yes          # kill the emulator and remove ~/.andro
andro logcat               # stream adb logcat
andro doctor               # check macOS/Apple Silicon, HVF, resolved paths
```

### Options (global)

| Flag | Default | Meaning |
|------|---------|---------|
| `--tv` | off | Use the Android TV profile (leanback). |
| `--api <N>` | `36` (Android 16) | Android API level. |
| `--device <P>` | `pixel` / `tv_1080p` | avdmanager device profile. |
| `--home <DIR>` | `$ANDRO_HOME` or `~/.andro` | Self-contained SDK location. |
| `--api`/`--device` per run | — | also via `ANDRO_API`, `ANDRO_HOME` env. |

`run` flags: `--no-launch` (install only), `--clean-after` (tear down afterwards).

## How it works

On first `run`/`up`, `andro` provisions `~/.andro` end to end:

1. downloads a **Temurin JDK 17** (no brew),
2. downloads the Android **command-line tools**, then installs `platform-tools`,
   `emulator` and the chosen **system image** (`google_apis` for phone,
   `android-tv` for `--tv`),
3. creates an AVD (`andro` or `andro-tv`),
4. boots the emulator (HVF) and waits for `sys.boot_completed`.

`run` then `adb install`s the APK, detects the new package (diffing
`pm list packages`), resolves its launchable activity for the profile's category
(`LAUNCHER` / `LEANBACK_LAUNCHER`) and starts it with `am start`.

Everything is contained in `~/.andro`, so cleanup is just removing that folder.

## Development

```sh
cargo test                                   # unit + hermetic CLI tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
# full end-to-end (needs macOS + ~2GB download):
ANDRO_E2E_APK=/path/app.apk cargo test -- --ignored e2e
```

Releases are cut by **semantic-release** from Conventional Commits; macOS
binaries are attached to each GitHub release.

## License

MIT © maxgfr
