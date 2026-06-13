# andro

Run Android apps on phone **or smart TV** — from your macOS command line, on a
fast, **disposable** Android emulator. No Android Studio, nothing installed
system-wide — andro downloads a self-contained JDK + Android SDK into `~/.andro`,
and `andro clean` wipes everything.

```sh
andro run app.apk          # boot a phone emulator, install, launch
andro run app.xapk         # split-APK bundles (.xapk/.apks/.apkm) and split dirs too
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
brew install maxgfr/tap/andro   # Homebrew (macOS) — recommended
# or:
cargo install --path .          # build from source
# or grab a prebuilt binary from the GitHub Releases
```

## Usage

```sh
# run & install
andro run <target>         # provision (first run) → boot → install → launch
andro --tv run <target>    # Android TV profile (TV image + leanback launcher)
andro install <target>     # install only (no launch)
andro launch <pkg>         # launch an already-installed package
andro uninstall <pkg>      # remove a package
andro clear <pkg>          # wipe an app's data — fast pristine first-run
andro list                 # list installed (third-party) packages  [--system --json]
andro test <pkg>           # run instrumentation tests (am instrument)

# inspect & interact
andro open <url>           # fire a VIEW intent — deep-link / URL-scheme testing
andro screenshot [out.png] # capture the screen (defaults to ./screenshot.png)
andro record [out.mp4]     # record the screen  [--time <secs>, max 180]
andro shell <args…>        # passthrough to `adb shell` (e.g. andro shell getprop)
andro applog <pkg>         # stream just this app's logcat  [--clear --crash]
andro logcat               # stream the whole device log  [--dump --clear]
andro push <local> <rem>   # copy a file onto the device
andro pull <rem> [local]   # copy a file off the device (→ ~/.andro/tmp)
andro reverse <port>       # bridge a Mac host port into the emulator  [--list --remove-all]

# lifecycle & housekeeping
andro up                   # provision + boot
andro wait                 # block until the emulator is ready (scriptable)
andro status               # running/booted? exits 0/2/3  [--json]
andro stop                 # stop the emulator, keep ~/.andro
andro autoclean            # reclaim temp/cache/logs  [--reset --images --deep --dry-run --yes]
andro clean --yes          # kill the emulator and remove ~/.andro
andro doctor               # check macOS/Apple Silicon, HVF, paths, adb port
andro completions <shell>  # print a bash/zsh/fish completion script
```

`<target>` for `run`/`install` is a single `.apk`, a **directory of split APKs**
(installed with `install-multiple`), or a **`.xapk` / `.apks` / `.apkm` bundle**
(unzipped, then installed). Apps install with permissions pre-granted (`adb -g`).

### Options (global)

| Flag | Default | Meaning |
|------|---------|---------|
| `--tv` | off | Use the Android TV profile (leanback). |
| `--profile <P>` | `phone` | Form factor: `phone` or `tv` (shorthand for `--tv`). |
| `--playstore` | off | Use a Play Store image (`google_apis_playstore`); phone only. |
| `--api <N>` | `36` (Android 16) | Android API level. |
| `--device <P>` | `pixel` / `tv_1080p` | avdmanager device profile. |
| `--home <DIR>` | `$ANDRO_HOME` or `~/.andro` | Self-contained SDK location. |
| `--snapshot` / `--fast` | off | Persist a quickboot snapshot for ~2-5s reboots (state survives until `clean`). |
| `--no-window` | off | Run the emulator headless (e.g. on CI). |
| `--cores <N>` / `--memory <MB>` | auto | Emulator CPU / RAM overrides. |
| `--api`/`--device` per run | — | also via `ANDRO_API`, `ANDRO_HOME` env. |

`run`/`install` flags: `--downgrade` (allow an older version, `adb -d`).
`run` also takes `--no-launch` (install only) and `--clean-after` (tear down afterwards).

### Config file

Drop a `config.toml` in your andro home (`~/.andro/config.toml`, or
`$ANDRO_HOME/config.toml`) to set defaults. Precedence is **CLI flag > `ANDRO_*`
env > config file > built-in**:

```toml
profile  = "tv"      # phone | tv
api      = 34
device   = "tv_4k"
playstore = false
snapshot = false     # = --snapshot/--fast
no_window = false
cores    = 6
memory   = 4096
```

### Shell completions

```sh
andro completions zsh  > "${fpath[1]}/_andro"        # zsh
andro completions bash > /usr/local/etc/bash_completion.d/andro
andro completions fish > ~/.config/fish/completions/andro.fish
```

## Housekeeping: `autoclean`

`clean` is the nuclear option (removes all of `~/.andro`). `autoclean` reclaims
space **without** re-downloading the SDK/JDK/active image:

```sh
andro autoclean              # SAFE: wipe tmp/, caches, stale locks, rotate the log
andro autoclean --dry-run    # preview what would be freed, delete nothing
andro autoclean --reset --yes  # also FACTORY-RESET the device (wipe userdata; no re-download)
andro autoclean --images --yes # also prune system images no AVD references (re-download on next use)
andro autoclean --deep --yes   # = --reset + --images
andro autoclean --legacy --yes # remove pre-containment residue from ~/.android (only if andro-only)
```

It refuses while the emulator is running (`andro stop` first), reports bytes
freed per category, and **never deletes files it didn't create** (e.g. a
screenshot you saved into the home dir is listed and skipped).

## Self-contained — nothing left behind

Everything andro creates lives under `~/.andro` (its own JDK, SDK, AVDs, **and**
the tools' `.android` home and all temp files), so `andro clean` truly leaves
nothing behind. To make that hold even when Android Studio is open, andro runs
its **own adb server on a dedicated port** (so a foreign `adb` server on the
default 5037 can't capture andro's keys). One consequence: andro's emulator
won't show up in a plain `adb devices` on 5037 — use `andro shell` / `andro
status`, and `andro doctor` prints the port. Older installs may have leaked a few
files into `~/.android`; `andro doctor` flags it and `andro autoclean --legacy`
cleans it (only when that dir holds nothing but andro's own residue).

## How it works

On first `run`/`up`, `andro` provisions `~/.andro` end to end:

1. downloads a **Temurin JDK 17** (no brew),
2. downloads the Android **command-line tools**, then installs `platform-tools`,
   `emulator` and the chosen **system image** (`google_apis` for phone,
   `android-tv` for `--tv`),
3. creates an AVD (`andro` or `andro-tv`),
4. boots the emulator (HVF), waits for `sys.boot_completed`, then waits until
   PackageManager actually answers (so the first install can't race the boot).

`run` then installs (`adb install -r -g -t`, or `install-multiple` for split
APKs/bundles — runtime permissions pre-granted), detects the new package (diffing
`pm list packages`), resolves its launchable activity for the profile's category
(`LAUNCHER` / `LEANBACK_LAUNCHER`) and starts it with `am start`.

Everything is contained in `~/.andro`, so cleanup is just removing that folder.

## Known limitations

- **`.aab` (Android App Bundles)** can't be installed by `adb` — build an APK or
  `.apks` with `bundletool` first, then `andro run` it.
- **Apps requiring strong Play Integrity / SafetyNet** won't pass attestation on an
  emulator even with `--playstore`; that flag only helps apps that check for the
  Play Store's *presence* (real GMS), not hardware-attested integrity.

## Development

```sh
cargo test                                   # unit + hermetic CLI tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
# full end-to-end (needs macOS + ~2GB download):
ANDRO_E2E_APK=/path/app.apk cargo test -- --ignored e2e
```

Releases are cut by **semantic-release** from Conventional Commits; macOS
binaries (arm64 + x86_64) are attached to each GitHub release, and the
[Homebrew tap](https://github.com/maxgfr/homebrew-tap) (`maxgfr/tap/andro`)
auto-updates its formula from the latest release within a day.

## License

MIT © maxgfr
