<div align="center">

# Data Mining

### Open, run and edit IBM SPSS Modeler 19 streams, without Modeler. An 18 to 19 MB download.

**A data-mining workbench written in Rust, for macOS (Apple silicon) and Windows. Its results are compared with Modeler's own output on 1,601 recorded streams.**

[![License: PolyForm Noncommercial 1.0.0](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-blue.svg)](LICENSE)
[![Latest release](https://img.shields.io/github/v/release/witold-andelie/data-mining-software?include_prereleases&label=download)](https://github.com/witold-andelie/data-mining-software/releases/tag/v0.6.0)
![Rust 1.95](https://img.shields.io/badge/rust-1.95.0-orange.svg)
![Platforms](https://img.shields.io/badge/platforms-macOS%20(Apple%20silicon)%20%7C%20Windows%20x64-blue.svg)
![Version](https://img.shields.io/badge/version-0.6.0-informational.svg)
![Status](https://img.shields.io/badge/status-active%20development-green.svg)

**[Download](#download) · [First steps](#first-steps) · [Why it is different](#why-data-mining) · [Results](#results-at-a-glance) · [FAQ](#faq) · [Documentation](#documentation)**

<img src="crates/ui/tests/snapshots/workbench_light_in_window_menu.png" width="760" alt="The main window on a synthetic stream: node palette, canvas, properties panel and results area">

<sub>The main window on a generated stream (the screenshot test's reference image: no IBM or team data). On macOS the menu is the native menu bar.</sub>

</div>

> [!IMPORTANT]
> Get Data Mining only from this repository's [Releases page](https://github.com/witold-andelie/data-mining-software/releases).
> It is **free for individuals and educational institutions; no commercial use** ([PolyForm Noncommercial 1.0.0](LICENSE),
> see [License](#license)). It is not affiliated with, endorsed by or derived from IBM. "IBM", "SPSS" and "Modeler"
> are trademarks of their owners and are used here only to describe compatibility.

Source code, regression suite and verification records of **Data Mining**, a data-mining workbench in Rust that targets
the single-machine feature set of IBM SPSS Modeler 19. It opens existing Modeler `.str` streams, runs them, and saves
streams in its own reviewable text format (`.dms`). Every number in this README is produced by a command in this
repository (`cargo xtask regress`, `parity`, `perf`, `ci`) and carries the date it was measured.

## Quick start

### Download

Version 0.6.0 is a pre-release. Pick your system, download, and check the file against its `.sha256` next to it.

| System | Download | Size | First start |
|---|---|---|---|
| **macOS** (Apple silicon, macOS 11 or later) | [Data-Mining-0.6.0-macos-arm64.dmg](https://github.com/witold-andelie/data-mining-software/releases/download/v0.6.0/Data-Mining-0.6.0-macos-arm64.dmg) | 19.4 MB | Open the `.dmg`, drag **Data Mining** to Applications. macOS blocks the first start (ad hoc signed, not notarized): try to open it once, then **System Settings > Privacy & Security > Open Anyway**. |
| **Windows** (x64) | [Data-Mining-0.6.0-windows-x64.zip](https://github.com/witold-andelie/data-mining-software/releases/download/v0.6.0/Data-Mining-0.6.0-windows-x64.zip) | 18.1 MB | Unzip, run `dms.exe`. Nothing else to install. Not code-signed, so SmartScreen may warn: **More info > Run anyway**. |

All files, with their checksums, are on the [release page](https://github.com/witold-andelie/data-mining-software/releases/tag/v0.6.0)
(GitHub shows sizes in MiB there: 18.5 and 17.2). Details, the steps verified on macOS 26.5.1 and the terminal
alternative to *Open Anyway*: [Installation](docs/installation.md#opening-the-downloaded-app-first-time).

### First steps

- **Open a stream:** File > Open… (a Modeler `.str` or our `.dms`), or pass the file on the command line (`dms.exe
  stream.str` on Windows). No stream at hand? `dms.exe --synthetic 24` opens a generated one that needs no data files.
- **Run it:** press ⌘R (Ctrl+R on Windows). If the stream names data files saved on another machine, add the folder that
  holds them with File > Add Data Folder… (the app searches by file name).
- **Without a window:** `dms-cli run stream.str --out results/` (inside the macOS app; `dms-cli.exe` in the Windows
  download). See [Using Data Mining](docs/using.md#command-line-tool).
- **Python nodes (optional):** `python3 -m pip install pyarrow pandas`, plus the libraries of the nodes you use (see
  [Python nodes](docs/using.md#python-nodes)).

### Build from source (optional)

<details>
<summary><strong>macOS (Apple silicon) and Windows (x64)</strong></summary>

**macOS**

```sh
xcode-select --install          # compiler and linker; skip if already installed
brew install unixodbc           # a library the app links (needs Homebrew, https://brew.sh)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh     # Rust; the right version is picked automatically
git clone https://github.com/witold-andelie/data-mining-software.git
cd data-mining-software
cargo run -p dms --release      # builds and starts the app
```

**Windows**: install Rust from <https://rustup.rs> (it needs the Microsoft C++ Build Tools; the installer offers them)
and [Git](https://git-scm.com), then in a terminal:

```powershell
git clone https://github.com/witold-andelie/data-mining-software.git
cd data-mining-software
cargo run -p dms --release      # builds and starts the app
```

The first build compiles all dependencies (about 7 minutes on the Windows laptop this was checked on, with two build
jobs). The right Rust version (1.95.0) is selected by `rust-toolchain.toml`. Packaging commands:
`cargo xtask bundle-mac` (`.app` and `.dmg`) and `cargo xtask bundle-windows` (the `.zip`). More:
[Installation](docs/installation.md).

</details>

## Why Data Mining

- **Opens Modeler streams without Modeler.** Decodes `.str` files; nodes and settings the app does not understand are
  kept exactly as saved and drawn with a dashed outline, so nothing is lost.
- **Small and self-contained.** The macOS disk image is **19.4 MB** and the Windows download is **18.1 MB** (one
  **32.8 MB** executable plus a 16.0 MB command-line tool), against **5.1 GB** for the Modeler 19 installation it replaces
  on the machine the oracles were recorded on. No Java runtime, no Spark, no database server and no Python are needed
  to open, edit and run streams; Python is only for the optional Python nodes.
- **Runs them with verified results.** Each stream in a regression suite is run and every terminal node's input is
  compared with Modeler's own batch export.
- **All 85 MVP nodes** across the standard palettes (Sources, Record Ops, Field Ops, Graphs, Modeling, Output, Export),
  plus all 24 "later" nodes, including the complete Python palette, and the six standard nodes Modeler's licence tier
  greys out in the oracle's installation.
- **Our own training algorithms,** from published documentation: C5.0, CHAID, C&R Tree, QUEST, linear, logistic and
  generalized linear regression, Cox regression, discriminant analysis, neural networks, SVM, KNN, Bayes Net, K-Means,
  TwoStep, Apriori, time series and more.
- **A visual editor and results viewer** (egui, no WebView): palette, canvas, generated settings forms, undo/redo,
  autosave and crash recovery; virtual-scrolling tables, statistics documents, model summaries and charts.
- **A Python bridge** for Extension nodes and Python modelling nodes, run in your own Python.
- **English and Simplified Chinese** interface, following the system language.

More on the design and what it contributes: [About](docs/about.md).

## Results at a glance

| Question | Finding |
|---|---|
| Do results match Modeler? | A regression suite of **1,601 streams** is compared with Modeler's own batch output: **1,469 match exactly, 91 match to statistical parity, 41 are documented known gaps and 0 regress** (Windows, 2026-11-23, version 0.6.0). |
| Same answers on Windows and macOS? | Yes: on 2026-11-23 the full regression on an Apple-silicon Mac classified all 1,601 streams exactly as on Windows. |
| What is statistical rather than exact? | Models Modeler makes unreproducible (undocumented random numbers or optimizer stopping rule): MLP neural network, Kohonen, LSVM, Random Trees, Time Series, Sequence, Auto Cluster / Classifier / Numeric. Nothing statistical is presented as exact. |
| What is not covered? | 41 documented known gaps and a handful of missing nodes and features: [Known gaps](docs/known-gaps.md). |
| How big is it? | Download 18.1 MB (Windows) and 19.4 MB (macOS); `dms.exe` 32.8 MB, `Data Mining.app` 44.7 MB. A started app held about 300 MB (Windows working set) and 128 MB (macOS resident) on a generated 24-node stream. |

Node coverage, generated from the node catalog by `cargo xtask parity` ([checklist](docs/parity-checklist.md)):

| Scope | Implemented | Total | Notes |
|---|---|---|---|
| MVP | 85 | 85 | The standard single-machine palettes |
| Later | 24 | 24 | Python palette, extension nodes, Cox, GLMM, Temporal Causal Model and Streaming TS done |
| Licence-blocked | 6 | 6 | Standard nodes the oracle's Modeler licence greys out (SVM, KNN, Bayes Net, Discriminant, GenLin, SLRM), so they cannot be verified. All six are implemented from the public algorithms and checked against models Modeler saved in demo streams |
| Out of scope | 0 | 34 | IBM ecosystem, big data, in-database mining, Statistics integration |

The full results table (exact and statistical model lists, Bayes Net, Cox, GenLin, SVM, KNN, GLMM, TCM, Discriminant
and SLRM checks, editor speed, time-series Expert Modeler), how to reproduce it and its limitations:
[Results and verification](docs/verification.md).

## FAQ

<details>
<summary><strong>Do I need IBM SPSS Modeler?</strong></summary>

No. Data Mining opens and runs `.str` streams without Modeler installed. Modeler was used only to record the expected
answers the regression suite compares against; it is never linked, bundled or distributed.

</details>

<details>
<summary><strong>Which systems are supported?</strong></summary>

macOS on Apple silicon (macOS 11 or later) and Windows x64. The Windows desktop app has not yet had a full manual pass;
the Windows build passes `cargo xtask ci` and the full regression.

</details>

<details>
<summary><strong>Why does macOS or Windows warn me when I start it?</strong></summary>

The programs are not signed with a paid developer identity: the macOS app is signed ad hoc and not notarized, and the
Windows programs are not code-signed. Check the download against its `.sha256` file, then follow the first-start step in
the [download table](#download). Both steps are described in [Installation](docs/installation.md#opening-the-downloaded-app-first-time).

</details>

<details>
<summary><strong>Will my results be identical to Modeler's?</strong></summary>

For 1,469 of 1,601 recorded streams, yes, record by record. For 91 the models are equivalent in distribution but not
numerically identical (random numbers, optimizer stopping rules). 41 are documented gaps with their reasons. See
[Results and verification](docs/verification.md) and [Known gaps](docs/known-gaps.md).

</details>

<details>
<summary><strong>Is it a copy of Modeler?</strong></summary>

No. Every algorithm follows IBM's published *Algorithms Guide*, the manuals and the open literature; Modeler's programs are
never decompiled or opened, and no GPL code is read. Modeler is only a black-box oracle. See [Clean room](#clean-room).

</details>

<details>
<summary><strong>What do I need for Python nodes?</strong></summary>

Your own Python 3.9 or later with `pyarrow` and `pandas`, plus the libraries of the nodes you use. Python nodes run as a
separate process in your Python; no Python is bundled. See [Python nodes](docs/using.md#python-nodes).

</details>

<details>
<summary><strong>Can I use it at work?</strong></summary>

Not for commercial use. The licence is free for individuals and educational institutions and other noncommercial uses;
commercial licences are granted by the author on request. See [License](#license).

</details>

<details>
<summary><strong>I found a bug. What should I do?</strong></summary>

Open an [issue](https://github.com/witold-andelie/data-mining-software/issues) with the version (the release name),
your system, the steps to reproduce and the error output. For a wrong result, say which stream and which node and,
if you can, what Modeler gives.

</details>

## Documentation

- [Installation](docs/installation.md): downloads, building from source, the macOS app, first start, app options.
- [Using Data Mining](docs/using.md): the desktop app and shortcuts, Python nodes, the command-line tool, file formats.
- [Results and verification](docs/verification.md): the full results table, data and oracles, how to reproduce, limitations.
- [Known gaps and their impact](docs/known-gaps.md): every difference from Modeler, with the reason.
- [About](docs/about.md): overview, main contributions, the clean-room method.
- [Development](docs/development.md): repository structure, build gates, conventions.
- [Statistical parity](docs/statistical-parity.md), [node checklist](docs/parity-checklist.md), [CLEM reference](docs/clem/reference.md), [node settings as observed](docs/formats/nodes/common.md).

## Development

Rust 1.95.0 (`rust-toolchain.toml`), edition 2024. New dependencies must be permissively licensed (MIT, Apache-2.0,
BSD, ISC, Zlib, BSL-1.0, Unicode-3.0, CC0; MPL-2.0 only for test-only dependencies) and pinned in the workspace
`Cargo.toml`. Run `cargo xtask ci` (fmt, clippy `-D warnings`, all tests, layers, parity) before every commit. Commits
follow Conventional Commits. Repository structure, the full command list and the rules (never crash, layering,
everything is a command): [Development](docs/development.md).

## Clean room

Every modelling algorithm in this project is implemented and trained by us. None of it is reverse-engineered from IBM
SPSS Modeler, and no Modeler code is reused.

- **We never look inside Modeler.** Its jars, DLLs and executables are never decompiled, disassembled or opened. C5.0 is
  RuleQuest's proprietary algorithm; the C5.0/See5 source has not been consulted either.
- **Only public sources:** IBM's published *Algorithms Guide*, the user manuals, the *Python Scripting and Automation
  Guide*, the CLEM reference, the PMML standard and the open literature. Each algorithm module cites the chapter and
  page it implements.
- **Modeler is only a black-box oracle:** we prepare input data, let Modeler train and score in batch mode, and compare
  its output with ours cell by cell.
- **No GPL code and no IBM assets.** Nothing from IBM, the team's data or Modeler outputs is committed or distributed.

The full statement is in [About](docs/about.md#clean-room).

## Citation

If you use the software in research or teaching, please cite it as:

```bibtex
@misc{ma2026datamining,
  author = {Ma, Wentao},
  title  = {Data Mining: a {Rust} workbench for {SPSS} {Modeler} 19 streams},
  note   = {Software, version 0.6.0; code at https://github.com/witold-andelie/data-mining-software},
  year   = {2026}
}
```

## License

The code is released under the **[PolyForm Noncommercial License 1.0.0](LICENSE)** (SPDX identifier
`PolyForm-Noncommercial-1.0.0`). In short:

- **Free to use, study, modify and share** for any noncommercial purpose. That explicitly includes personal use
  (research, experiments, study, hobby projects without an anticipated commercial application) and use by
  educational institutions, public research organizations, charities, public safety or health organizations,
  environmental protection organizations and government institutions, regardless of how the work is funded.
- **Not allowed without a separate licence:** commercial use, for example building it into a company's products or
  services, or using it for paid work for clients.
- If you share the software or a changed version, you must pass on the licence text (or its URL) and the line
  beginning `Required Notice:` in the [`LICENSE`](LICENSE) file.
- **Commercial licences** are granted by the author on request; contact the repository owner on GitHub.

This is a source-available licence, not one approved by the Open Source Initiative, so the project is not "open
source" in that sense. The summary above is for convenience; the licence text governs. Third-party dependencies keep
their own permissive licences (list them with `cargo metadata`).

**Bundled ODBC libraries.** The macOS app ships two LGPL libraries as an approved exception to the permissive-only rule:
`libodbc` from unixODBC (LGPL-2.1-or-later) and `libltdl` from GNU libtool (LGPL-2.0-or-later), which `libodbc` needs.
They are unmodified, dynamically linked and kept as separate files in `Contents/Frameworks`, so they can be replaced.
Their licence texts and source locations are in `Contents/Resources/Licenses/` inside the app.

## Acknowledgements

The author thanks [IBM](https://github.com/IBM) for publishing the documentation this work follows (the SPSS Modeler
*Algorithms Guide*, user manuals, scripting guide and CLEM reference). This does not imply any affiliation with, or
endorsement by, IBM.

The author also thanks the [REA](https://github.com/morluto/rea) project (*Reverse Engineer Anything*, by morluto,
MIT licence).

The work rests on other public material: the Data Mining Group's PMML standard, ITU-T X.891 (Fast Infoset), the
Rust ecosystem, and the egui project and its libraries.

