# Installation

Downloads, building from source, the macOS app and its first start. The shortest path is in the [README](../README.md#quick-start).

## Quick start in detail

**macOS (Apple silicon):** download the disk image **Data-Mining-0.6.0-macos-arm64.dmg** (19.4 MB; the
[release page](https://github.com/witold-andelie/data-mining-software/releases/tag/v0.6.0), a pre-release, shows sizes in MiB: 18.5) together with its
`.sha256` file, open it and drag **Data Mining** to Applications. The app is signed ad hoc and not notarized, so
macOS blocks it the first time; see [Opening the downloaded app](#opening-the-downloaded-app-first-time).

**Windows (x64):** download **Data-Mining-0.6.0-windows-x64.zip** (18.1 MB, shown as 17.2 MiB on the page) from the same release, unzip it and run
`dms.exe`. Nothing else has to be installed (the C runtime is linked in). The programs are not code-signed, so Windows
SmartScreen may warn you the first time: check the `.sha256` file, then choose **More info > Run anyway**.

**Building it yourself:** the steps below build the app from source. They take a few commands and no configuration.

**macOS** (Apple silicon, macOS 11 or later)

```sh
xcode-select --install          # compiler and linker; skip if already installed
brew install unixodbc           # a library the app links (needs Homebrew, https://brew.sh)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh     # Rust; the right version is picked automatically
git clone https://github.com/witold-andelie/data-mining-software.git
cd data-mining-software
cargo run -p dms --release      # builds and starts the app
```

**Windows** (x64)

1. Install Rust from <https://rustup.rs>. It needs the Microsoft C++ Build Tools; the installer tells you if they are
   missing and offers to install them. Install [Git](https://git-scm.com) if you do not have it.
2. In a terminal:

   ```powershell
   git clone https://github.com/witold-andelie/data-mining-software.git
   cd data-mining-software
   cargo run -p dms --release      # builds and starts the app
   ```

The first build compiles all dependencies: it took about 7 minutes on the Windows laptop this was checked on, with
two build jobs (the default uses every core and is faster). Later starts take seconds. The right Rust version (1.95.0) is
selected by `rust-toolchain.toml` and installed by `rustup` on the first command.

**Then:**

- **Open a stream:** File > Open… (a Modeler `.str` or our `.dms`), or pass it on the command line:
  `cargo run -p dms --release -- path/to/stream.str`. No stream at hand? `cargo run -p dms --release -- --synthetic 24`
  opens a generated one that needs no data files.
- **Run it:** press ⌘R (Ctrl+R on Windows). If the stream names data files saved on another machine, add the folder that
  holds them with File > Add Data Folder… (the app searches by file name).
- **A double-clickable app (macOS):** `cargo xtask bundle-mac` writes `target/dist/Data Mining.app` and a `.dmg`; drag the
  app to Applications (an app you build yourself opens directly; a downloaded copy needs the steps in
  [Opening the downloaded app](#opening-the-downloaded-app-first-time)).
- **Python nodes (optional):** `python3 -m pip install pyarrow pandas`, plus the libraries of the nodes you use (see
  [Python nodes](using.md#python-nodes)).
- **Without a window:** `cargo run -p dms-cli -- run stream.str --out results/` (see [Command-line tool](using.md#command-line-tool)).

More detail, requirements and options: [Installation in detail](#installation-in-detail).

## Installation in detail

| | |
|---|---|
| macOS | Apple silicon (`aarch64-apple-darwin`), macOS 11 or later |
| Windows | x64 |
| Rust (to build) | 1.95.0, selected automatically by `rust-toolchain.toml` |
| Python (optional) | 3.9 or later with `pyarrow` and `pandas`, only for Python nodes |
| ODBC | The macOS app bundles the unixODBC driver manager (see [License](../README.md#license)). Building from source needs it installed (`brew install unixodbc`). Database nodes also need an ODBC driver for your database |

### The macOS app

1. Build the disk image: `cargo xtask bundle-mac` (needs the steps in [Quick start](../README.md#quick-start)).
2. Open the `.dmg` and drag **Data Mining** to Applications.
3. The first time, macOS blocks a downloaded copy (see below).
4. Double-clicking a `.str` or `.dms` file in Finder (or Open With) opens it in the app.
5. Optional: the command-line tool is inside the app; link it once with
   `sudo ln -sf "/Applications/Data Mining.app/Contents/MacOS/dms-cli" /usr/local/bin/dms-cli`.

`bundle-mac` builds the release binaries, copies the approved Homebrew ODBC libraries into the app (anything else
fails the build) and rewrites their references, adds the icon, our own `LICENSE` and the third-party licence files,
signs ad hoc and creates the disk image.

#### Opening the downloaded app (first time)

The app is signed ad hoc and not notarized, so macOS refuses a copy downloaded from the internet. On the Mac we tested
(macOS 26.5.1) `spctl --assess` reports `rejected` for it, as it does for any app without a developer identity and a
notarization ticket. Apple's own steps ([Open a Mac app from an unidentified developer](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac)):

1. Try to open **Data Mining** once from Applications and dismiss the warning.
2. Open **System Settings > Privacy & Security**, scroll to **Security**, and click **Open Anyway** next to
   *Data Mining*. Confirm, and enter your password if asked.

From macOS 15 (Sequoia) on, right-click > Open no longer bypasses this for unsigned or un-notarized apps (Apple's
documentation says it still works on macOS 14 and earlier; we have not tested that). If you prefer the terminal, remove the download mark yourself after checking the `.sha256` file:
`xattr -dr com.apple.quarantine "/Applications/Data Mining.app"`. Verified on macOS 26.5.1 with the 0.6.0 disk image:
right-click > Open did not let the app start, **Open Anyway** did, and it then opened normally.

### Useful app options

| Option | Effect |
|---|---|
| `<file.str\|file.dms>` | Open this stream on start |
| `--run` | Run the opened stream once it is open; the window stays open with the results |
| `--data-dir <dir>` | Extra folder searched for data files the stream names (repeatable) |
| `--report <file>` | With `--run`: write a plain-text run summary (counts and result kinds, no data values), then quit without saving |
| `--synthetic <n>` | Open a generated stream with `n` nodes (no data needed) |
| `--frame-time` | Show the CPU time of each frame in the status bar |
