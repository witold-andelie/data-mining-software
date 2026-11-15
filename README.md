# Data Mining (data-mining-software)

A lightweight data-mining workbench written in Rust that targets the single-machine
feature set of IBM SPSS Modeler 19: visual stream building, data preparation, modelling,
evaluation and export. It opens existing Modeler `.str` stream files, runs them with results
checked against Modeler itself, and saves streams in its own reviewable text format (`.dms`).

The target platforms are Windows (x64) and Apple-silicon macOS. The user interface is native
Rust (egui): no Electron, Tauri, WebView or WebAssembly.

> **Internal project.** This repository is for team use and is not published. It is not
> affiliated with, endorsed by or derived from IBM. "IBM", "SPSS" and "Modeler" are trademarks
> of their owners and are used here only to describe compatibility.

---

## Contents

1. [Highlights](#highlights)
2. [Clean-room implementation](#clean-room-implementation)
3. [How close is it to Modeler?](#how-close-is-it-to-modeler)
4. [Installing and running](#installing-and-running)
5. [Using the desktop app](#using-the-desktop-app)
6. [Python nodes](#python-nodes)
7. [Command-line tool](#command-line-tool)
8. [File formats](#file-formats)
9. [Architecture](#architecture)
10. [Development](#development)
11. [Testing and verification](#testing-and-verification)
12. [Repository layout](#repository-layout)
13. [Documentation index](#documentation-index)
14. [Known limitations](#known-limitations)
15. [License](#license)

---

## Highlights

- **Opens Modeler streams.** Decodes `.str` files (a ZIP container holding Fast Infoset XML)
  without Modeler installed. Nodes and settings the application does not understand are kept
  exactly as saved and shown with a dashed outline, so nothing is lost.
- **Runs them with verified results.** A regression suite of **1,596 corpus streams** is
  compared record by record with Modeler's own batch output: 1,464 match exactly, 91 match to
  statistical parity, 41 are documented known gaps, and **0 regress** (macOS, November 2026).
- **All 85 MVP nodes implemented** across the standard palettes (Sources, Record Ops, Field
  Ops, Graphs, Modeling, Output, Export), plus 20 of 24 "later" nodes, including the complete
  Python palette.
- **Our own training algorithms.** C5.0, CHAID, C&R Tree, QUEST, linear and logistic
  regression, neural networks, K-Means, TwoStep, Apriori, time series and more are implemented
  from the published algorithm documentation and verified against Modeler as a black box.
- **Visual editor.** Add nodes from the palette, connect them (three gestures), edit settings
  in generated forms or in a floating window, undo/redo, copy/paste, autosave and crash
  recovery.
- **Results viewer.** Large tables with virtual scrolling, statistics documents, model
  summaries (trees, coefficients, rules), and charts for graph nodes.
- **Python bridge.** Extension nodes and Python modelling nodes (Random Forest, XGBoost,
  Gaussian Mixture, One-Class SVM, HDBSCAN, KDE, SMOTE, t-SNE, and user-written models) run in
  the user's own Python, exchanging data as Apache Arrow.
- **English and Simplified Chinese** user interface, following the system language by
  default.
- **Never crash.** Corrupt files, bad arguments and full disks produce errors you can act on,
  never a crash or lost work (enforced by lints, see [Development](#development)).

---

## Clean-room implementation

Every modelling algorithm in this project is implemented and trained by us. None of it is
reverse-engineered from IBM SPSS Modeler, and no Modeler code is reused.

- **We never look inside Modeler.** Its jars, DLLs and executables are never decompiled,
  disassembled or opened. C5.0 is RuleQuest's proprietary algorithm; we have not read or
  consulted the C5.0/See5 source code either.
- **Only public sources.** The implementation follows IBM's published *Algorithms Guide*
  (formulas), the user manuals, the *Python Scripting and Automation Guide* (node
  properties), the CLEM reference, the PMML standard (DMG) and the open literature. Each
  algorithm module's documentation cites the chapter and page it implements.
- **Modeler is only a black-box oracle.** We prepare input data, let Modeler train and score
  in batch mode, and use its output as the expected answer, compared cell by cell with ours.
  Where the documentation is silent, the behaviour is pinned down only by differential
  probes ("change one input, observe the output"), never by reading program internals.
- **The `.str` format** is learned the same way: save a stream twice in Modeler with one
  setting changed, compare the bytes, and record the finding in
  [`docs/formats/str.md`](docs/formats/str.md). The decoder implements only that document.
- **No GPL code.** GPL/LGPL/AGPL code (for example PSPP) is never read or copied. Permissively
  licensed third-party tools are used only to generate test files.
- **No IBM assets.** No IBM icons, wording or colours are used. IBM sample streams, team data
  and Modeler outputs live only in the git-ignored `corpus/` and `oracle/` folders and are
  never committed or distributed.

---

## How close is it to Modeler?

Trained models meet one of two levels of parity (details in
[`docs/statistical-parity.md`](docs/statistical-parity.md)):

| Level | Meaning | Models |
|---|---|---|
| **Exact** | The trained model and its scores match Modeler bit for bit (to printed precision) | C5.0, CHAID, C&R Tree and QUEST (without an overfit-prevention set), Tree-AS, Linear, Linear-AS, Regression, Logistic, Decision List, K-Means, TwoStep, Apriori, CARMA, PCA/Factor, Anomaly, Feature Selection, RBF neural network |
| **Statistical** | Modeler uses an undocumented random number generator or optimizer stopping rule that cannot be reproduced bit for bit. We implement the documented algorithm with our own seedable generator or a converged optimizer, and check that Modeler's result lies within the spread of our repeated runs | MLP neural network, Kohonen, LSVM, Random Trees, Time Series, Sequence, Auto Cluster, Auto Classifier, Auto Numeric, and C&R Tree / QUEST with an overfit-prevention set |

Node coverage, generated from the node catalog by `cargo xtask parity`
([`docs/parity-checklist.md`](docs/parity-checklist.md)):

| Scope | Implemented | Total | Notes |
|---|---|---|---|
| MVP | 85 | 85 | The standard single-machine palettes |
| Later | 20 | 24 | Python palette and extension nodes done; Streaming TS, GLMM, Cox and Temporal Causal Model remain |
| Licence-blocked | 0 | 6 | Standard nodes the oracle's Modeler licence greys out (SVM, KNN, Bayes Net, Discriminant, GenLin, SLRM), so they cannot be verified |
| Out of scope | 0 | 34 | IBM ecosystem, big data, in-database mining, Statistics integration |

The Python nodes have no Modeler oracle: their score field names follow Modeler's naming
conventions but are inferred (see [Python nodes](#python-nodes)).

---

## Installing and running

### Requirements

| | |
|---|---|
| macOS | Apple silicon (`aarch64-apple-darwin`), macOS 11 or later |
| Windows | x64 (the Windows build is maintained but tested last) |
| Rust (to build) | 1.95.0, selected automatically by `rust-toolchain.toml` |
| Python (optional) | 3.9 or later with `pyarrow` and `pandas`, only for Python nodes |
| ODBC | The macOS app bundles the unixODBC driver manager (see [Licence note](#licence-note-bundled-odbc-libraries)). Building from source, or running `dms-cli`, needs it installed: `brew install unixodbc`. Database nodes also need an ODBC driver for your database |

### Install the macOS app

1. Build the disk image (see below) or take `DataMining-<version>.dmg` from a teammate.
2. Open the `.dmg` and drag **Data Mining** to Applications.
3. The app is signed ad hoc for internal use and not notarized. The first time, right-click
   the app in Finder and choose **Open**, then confirm.
4. Double-clicking a `.str` or `.dms` file in Finder (or using Open With) opens it in the app.

### Build from source

```sh
git clone <this repository> data-mining-software
cd data-mining-software

cargo run -p dms --release          # the desktop app
cargo run -p dms-cli -- --help      # the command-line tool (prints usage)

cargo xtask bundle-mac              # macOS: target/dist/Data Mining.app and DataMining-<version>.dmg
```

`bundle-mac` builds the release binary, copies the Homebrew ODBC libraries it links into the
app (only approved ones; anything else fails the build), rewrites their references to point
inside the app, adds the icon and licence files, signs ad hoc and creates the disk image.

Useful app options (mainly for testing):

| Option | Effect |
|---|---|
| `<file.str\|file.dms>` | Open this stream on start |
| `--run` | Run the opened stream, then quit without saving |
| `--data-dir <dir>` | Extra folder searched for data files the stream names (repeatable) |
| `--report <file>` | With `--run`: write a JSON run report |
| `--synthetic <n>` | Open a generated stream with `n` nodes (no data needed) |
| `--frame-time` | Show the CPU time of each frame in the status bar |

---

## Using the desktop app

The window has a node **palette** on the left, the stream **canvas** in the middle, the
**properties** panel on the right and the **results** panel at the bottom. Each panel can be
hidden from the toolbar or the View menu.

### Building a stream

- **Add a node:** drag it from the palette onto the canvas, or double-click it in the
  palette. Nodes that are not implemented yet are greyed out, with the reason as a tooltip.
- **Connect two nodes**, in any of three ways:
  - drag from the small circle on the right edge of a node to the target node;
  - hold **Option** (Alt on Windows) and drag from the node itself;
  - right-click the node, choose **Connect From Here**, then click the target.
- **Move** nodes by dragging; drag on empty canvas to box-select; Shift adds to the
  selection; hold Space and drag to pan; zoom with the toolbar, the View menu or the
  shortcuts below.
- **Delete a connection:** click it and press Delete, or right-click it and choose
  **Delete Connection**.
- **Rename a node:** select it and press Return (F2 on Windows), or use the node menu.

### Editing settings

- **Double-click a node** to open its settings in a floating window at the pointer. The window
  has a **Run** button (Run to Here for nodes in the middle of a stream) and **Close**;
  Escape closes it.
- The **properties panel** shows the same form for the selected node.
- Forms are generated from each node's settings schema: checkboxes, numbers with ranges,
  choices, field pickers (filled from the fields that reached the node in the last run), file
  pickers, CLEM expressions with syntax checking, tables and code editors. Every change is one
  undo step.
- Nodes whose settings cannot be edited yet show their settings XML read-only.

### The node menu (right-click a node)

| Item | Action |
|---|---|
| Edit… | Open the settings window |
| Run | Run this terminal node |
| Run From Here | Run every terminal node downstream of this node |
| Run to Here | Run the stream up to this node and show its data |
| Connect From Here | Start a connection; click the target node |
| Disconnect | Remove all of this node's connections (one undo step) |
| Rename… | Edit the node's name in place |
| Cut, Copy, Paste, Duplicate, Delete | Clipboard and editing |

### Running and results

- **Run** (⌘R) runs the whole stream; **Run Selected** (⇧⌘R) runs the selected node and what
  it needs. Runs happen in the background with a progress bar; **Stop** (⌘.) cancels,
  including long model training.
- After a run, nodes are marked as succeeded or failed. The results panel has one tab per
  terminal node: data tables (virtual scrolling for millions of rows), output documents
  (Statistics, Data Audit, Matrix, Means, Analysis, ...), model summaries, and charts for
  graph nodes (Histogram, Distribution, Plot, Multiplot, Time Plot, Evaluation, Collection,
  t-SNE), with a Chart/Data toggle.
- A failed node shows why, and nodes that did not run because something upstream failed say
  which node to look at.
- **Data folders.** Streams often name data files on the machine that saved them. The app
  looks in the stream's own folder first, then in the folders added with **File > Add Data
  Folder…** (also in Settings), searching by file name.

### Saving, autosave and recovery

- Streams are saved in the native `.dms` format (⌘S, ⇧⌘S). Imported `.str` files are never
  overwritten; saving asks for a `.dms` name.
- Edits are autosaved in the background. After a crash or forced quit, the app offers to
  recover unsaved streams on the next start.

### Settings (⌘,)

- **Language:** System, English or 简体中文.
- **Data folders:** add or remove the extra search folders.
- **Python:** the Python executable for Python nodes (empty: find one automatically) and a
  **Check Python** button that shows the version and which packages are installed.

### Keyboard shortcuts

| Action | macOS | Windows |
|---|---|---|
| New / Open / Close | ⌘N / ⌘O / ⌘W | Ctrl+N / Ctrl+O / Ctrl+W |
| Save / Save As | ⌘S / ⇧⌘S | Ctrl+S / Ctrl+Shift+S |
| Undo / Redo | ⌘Z / ⇧⌘Z | Ctrl+Z / Ctrl+Shift+Z |
| Cut / Copy / Paste / Duplicate | ⌘X / ⌘C / ⌘V / ⌘D | Ctrl+X / Ctrl+C / Ctrl+V / Ctrl+D |
| Select all | ⌘A | Ctrl+A |
| Delete | ⌫ | Delete |
| Rename | ↩ | F2 |
| Run stream / Run selected | ⌘R / ⇧⌘R | Ctrl+R / Ctrl+Shift+R |
| Stop | ⌘. | Escape |
| Zoom in / out / to fit | ⌘= / ⌘- / ⌘0 | Ctrl+= / Ctrl+- / Ctrl+0 |
| Full screen | ⌃⌘F | F11 |
| Settings | ⌘, | — |

On macOS the menu bar is the native one; on Windows the same menus are drawn in the window.
Both are generated from one command table, so every menu item has a command id that the UI,
the command line and automation all use.

---

## Python nodes

Python nodes run in **your own Python**, as a separate process. No Python is bundled and
libpython is not linked. Data goes to Python and back as Apache Arrow files in a private
temporary folder; scripts see a pandas DataFrame. Full design:
[`docs/python-bridge.md`](docs/python-bridge.md).

### Setup

```sh
python3 -m pip install pyarrow pandas            # required by every Python node
python3 -m pip install scikit-learn              # Random Forest, GMM, One-Class SVM, KDE, t-SNE
python3 -m pip install xgboost                   # XGBoost Tree / Linear
python3 -m pip install hdbscan                   # HDBSCAN
python3 -m pip install imbalanced-learn          # SMOTE
brew install libomp                              # macOS only: OpenMP runtime xgboost needs
```

The app looks for Python in this order: the path chosen in Settings; `DMS_PYTHON`; on macOS
`/opt/homebrew/bin/python3` and `/usr/local/bin/python3` (apps opened from Finder get a short
`PATH`); then `python3` and `python` (`py -3`, `python`, `python3` on Windows). A configured
path that does not work is reported, never silently replaced. Missing Python or packages
produce a message with the exact install command, for example
`/usr/local/bin/python3 -m pip install xgboost`.

### Script API (Extension nodes)

| Name | Meaning |
|---|---|
| `modelerData` | The input as a pandas DataFrame (`None` in Extension Import). Integers use nullable `Int64`, reals `Float64`, text `string`; dates are `datetime.date`, times `datetime.time`, timestamps `datetime64[us]` |
| `modelerDataModel` | The input fields: a list of `{name, storage, measurement, role}` |
| `print(...)` | The script's text output (the Extension Output node's result) |

Import and Transform scripts leave their result in `modelerData`. Measurement levels, roles
and labels of fields that pass through unchanged are kept. Errors report the line of your
script. Example Extension Transform script:

```python
modelerData["ratio"] = (modelerData["income"] / modelerData["age"]).round(2)
```

### Node list

| Palette | Node | Library | Output |
|---|---|---|---|
| Sources | Extension Import | your script | data |
| Record Ops | Extension Transform | your script | data |
| Record Ops | SMOTE | imbalanced-learn (SMOTE / SMOTENC) | the original records, then synthetic records of the smaller target classes |
| Graphs | t-SNE | scikit-learn `TSNE` | a 2-D scatter, coloured by a chosen field or the target |
| Modeling | Random Forest | scikit-learn | nugget: `$R-<target>`, `$RP-<target>` |
| Modeling | XGBoost Tree | xgboost (`gbtree`) | nugget: `$XT-<target>`, `$XTP-<target>` |
| Modeling | XGBoost Linear | xgboost (`gblinear`) | nugget: `$XL-<target>`, `$XLP-<target>` |
| Modeling | Gaussian Mixture | scikit-learn | nugget: `$GMM-<model>` (cluster), `$GMMP-<model>` |
| Modeling | One-Class SVM | scikit-learn | nugget: `$O-<model>` (`T` = outlier), `$OP-<model>` |
| Modeling | HDBSCAN | hdbscan | nugget: `$HB-<model>` (cluster or `noise`), `$HBP-<model>` |
| Modeling | KDE | scikit-learn `KernelDensity` | nugget: `$KDE-<model>` (log density) |
| Modeling | Extension Model | your build and score scripts | nugget: the fields your score script adds |
| Output | Extension Output | your script | the printed text |
| Output | KDE Simulation | scikit-learn `KernelDensity` | records drawn from the density of the inputs |
| Export | Extension Export | your script | writes files itself; passes its input on |

### Python models and nuggets

Like in Modeler, running a Python build node trains the model and then places a **model
nugget** next to it, connected to the same input. The nugget scores every record with the
saved model. The model is stored inside the stream (a pickle attachment), so it survives
saving and reopening; training again updates the same nugget. Generating the nugget is one
undo step.

- Fields come from the Type node's roles by default (Target; Input and Both); tick
  **Explicit model fields** to choose them in the node.
- With a partition field, only the training partition is used for training (switchable).
- A continuous target is regressed; any other target is classified.
- **Extension Model:** the build script leaves its model in `modelerModel`; the score script
  receives `modelerModel` and adds fields to `modelerData`. The score script is saved with the
  model, so changing it requires running the build node again.

> **Security.** Running a stream runs the Python code in it, and loading a saved Python model
> unpickles it (which can also run code), with your permissions. This is the same trust
> model as Modeler: only run streams you trust.

---

## Command-line tool

`dms-cli` works without the desktop app:

```text
dms-cli xml <file.str>                 print the decoded stream document as XML
dms-cli inspect <file.str>             print nodes, links and attachments as JSON
dms-cli coverage [--json] <path>...    report which catalog nodes the .str files use
dms-cli convert <in.str> <out.dms>     import a stream and save it in the native format
dms-cli run <file.str|file.dms> --out <dir> [--data-dir <dir>]... [--terminal-order <uid,...>]
                                       run the stream; write the data reaching each terminal
                                       node to <dir>/<uid>.csv and a report to <dir>/run.json
```

Export nodes write under `<dir>/exports/`, never over the paths saved in the stream. Python
nodes use `DMS_PYTHON` or find a Python as described above.

---

## File formats

| Format | Read | Write | Notes |
|---|---|---|---|
| Modeler stream `.str` | ✓ | — | ZIP + Fast Infoset (ITU-T X.891); unknown nodes and settings are preserved; see [`docs/formats/str.md`](docs/formats/str.md) |
| Native stream `.dms` | ✓ | ✓ | TOML text, diff-friendly; attachments (models, extension settings) as base64; saved atomically |
| Delimited text, fixed-width | ✓ | ✓ | Var. File, Fixed File, Flat File export |
| SPSS Statistics `.sav` / `.zsav` | ✓ | ✓ | Our own reader and writer |
| Excel `.xlsx` | ✓ | ✓ | |
| SAS | ✓ | ✓ | |
| JSON, XML | ✓ | ✓ | |
| Databases (ODBC) | ✓ | ✓ | Needs an ODBC driver manager |
| PMML 4.3 | ✓ | — | Model nuggets in `.str` files score from their saved PMML |

---

## Architecture

The workspace is split into layers. A crate may depend only on crates in lower layers, and
nothing below the UI layer may use a GUI library; `cargo xtask layers` enforces both.

| Layer | Crate | Responsibility |
|---|---|---|
| L0 | `dms-types` | Storage types, measurement levels, roles, missing values, values, fields, tables, cancellation |
| L0 | `dms-strfmt` | `.str` container and Fast Infoset decoder |
| L0 | `dms-sav` | SPSS Statistics `.sav` / `.zsav` |
| L0 | `dms-pmml` | PMML 4.3: reading models and scoring them |
| L1 | `dms-stream` | Stream model (nodes, links, settings kept verbatim), reversible edits, the `.dms` format |
| L1 | `dms-clem` | CLEM expressions: parser, binding to a data model, evaluation |
| L2 | `dms-catalog` | Every Modeler 19 node and how stream nodes map to it |
| L2 | `dms-io` | Data files and databases |
| L2 | `dms-algo` | Training algorithms (trees, regression, clustering, neural networks, time series, ...) |
| L2 | `dms-python` | Python bridge: environment detection, subprocess, Arrow exchange, model training and scoring |
| L3 | `dms-exec` | Executor: plans a stream and runs its nodes; settings schemas; cancellation |
| L4 | `dms-engine` | Session, command registry, undo/redo, background jobs, autosave, saving Python models |
| L6 | `dms-ui` | egui canvas, panels, settings forms, results, charts, translations |
| L6 | `dms-macos-open` | Files macOS asks the app to open (the only crate allowed `unsafe`) |
| apps | `dms` | Desktop application (eframe, native menus on macOS) |
| apps | `dms-cli` | Command-line tool |
| tools | `xtask` | CI gates, parity checklist, corpus, oracle, regression, perf, bundling |

Design principles:

- **Everything is a command.** Every user-visible action is an engine command with an id
  (`file.save`, `node.connect`, `run.node`, ...). Menus, shortcuts, the canvas, the command
  line and automation all call commands by id; commands never open dialogs themselves.
- **Data types are runtime data.** Storage, measurement level, role, missing values and date
  baselines are never hard-coded.
- **Settings are kept verbatim.** Imported settings are preserved byte for byte unless the
  user changes them; forms read and write them through per-node schemas.
- **Long work is cancellable.** Execution and training check a shared cancel token; Python
  processes are killed on Stop.

More: [`docs/architecture.md`](docs/architecture.md) and [`docs/gui/DESIGN.md`](docs/gui/DESIGN.md).

---

## Development

### Toolchain and dependencies

- Rust 1.95.0 (`rust-toolchain.toml`), edition 2024.
- New dependencies must be permissively licensed (MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0,
  Unicode-3.0, CC0; MPL-2.0 only for test-only dependencies), pinned, and registered in
  [`docs/gui/DESIGN.md`](docs/gui/DESIGN.md) §9.1 and the workspace `Cargo.toml`.

### Common commands

```sh
cargo xtask ci                 # fmt, clippy -D warnings, all tests, layers, parity: run before every commit
cargo xtask layers             # check the dependency layers
cargo xtask parity [--check]   # regenerate docs/parity-checklist.md from the catalog
cargo xtask perf               # canvas frame-time budget (200 nodes, p95 <= 4 ms)
cargo xtask bundle-mac         # build the .app and .dmg (macOS)

cargo xtask corpus verify      # check the local corpus against xtask/corpus.tsv (SHA-256)
cargo xtask corpus sync <label> <dir>
cargo xtask test-corpus        # corpus-backed tests (behind each crate's `corpus` feature)
cargo xtask regress [--label <label>]... [--jobs <n>] [--via-dms] [--out <file>]
cargo xtask compare ...        # compare one stream's output with its oracle
cargo xtask oracle <label> <path>   # Windows + Modeler only: record expected answers
cargo xtask probe <name>            # Windows + Modeler only: differential format probe
```

### Rules

- **Never crash.** Non-test code may not use `unwrap`, `expect`, `panic!`, `unreachable!`,
  `todo!` or `unimplemented!` (workspace lints), and `unsafe` is forbidden outside
  `dms-macos-open`. Numbers from files and arguments are treated as hostile: checked
  indexing, character-boundary slicing, checked or saturating arithmetic, and size limits
  before allocating. Recursion and loops are bounded. Every crash fix comes with a test that
  panicked before the fix; parsers have mutation fuzz tests.
- **Errors are actionable.** Errors carry context and say what the user can do.
- **Commits** follow Conventional Commits (`feat(exec): ...`, `fix(ui): ...`).
- Full contributor rules: [`AGENTS.md`](AGENTS.md).

---

## Testing and verification

| Kind | Where | What |
|---|---|---|
| Unit and integration tests | each crate's `src/` and `tests/` | Run by `cargo xtask ci` |
| Corpus tests | `--features corpus`, `cargo xtask test-corpus` | Real streams and data from the local corpus |
| Regression | `cargo xtask regress` | Runs every corpus stream with an oracle and compares each terminal node's input with Modeler's export; `--via-dms` first converts to `.dms` and back. The macOS baseline is [`docs/mac/mac-regress.tsv`](docs/mac/mac-regress.tsv) |
| Screenshot tests | `crates/ui/tests/snapshots.rs` | Off-screen rendering (egui_kittest + wgpu) of synthetic streams; reference images are updated only after review |
| Interaction tests | `crates/ui/tests/*.rs` | Real pointer and keyboard events: connecting, menus, editing, undo, settings forms, translations |
| Python tests | `crates/python/tests`, `crates/engine/tests/python_nodes.rs` | Run against the local Python; skipped with a message when Python or a package is missing |

### Corpus and oracle

`xtask/corpus.tsv` pins every corpus file by label, relative path and SHA-256. The files
themselves (IBM sample streams, team streams and data) and the oracle (Modeler's exported
models and terminal-node inputs) stay in the git-ignored `corpus/` and `oracle/` folders.
Oracles are recorded on Windows with Modeler in batch mode (`cargo xtask oracle`, needs
`DMS_MODELER_HOME`); see [`docs/oracle.md`](docs/oracle.md). Modeler is used only to
produce expected answers; it is never linked, bundled or distributed.

---

## Repository layout

```text
apps/
  dms/            desktop application (icon, macOS bundle metadata)
  dms-cli/        command-line tool
crates/
  types/ strfmt/ sav/ pmml/          L0
  stream/ clem/                      L1
  catalog/ io/ algo/ python/         L2
  exec/                              L3
  engine/                            L4
  ui/ macos-open/                    L6
docs/
  architecture.md, gaps.md, models.md, oracle.md, python-bridge.md, statistical-parity.md, ...
  formats/        .str format and node settings findings
  gui/            UI design and version handoffs
  mac/            macOS regression baseline and reports
xtask/            developer tasks; corpus.tsv; oracle scripts and probes
corpus/, oracle/  local only, git-ignored
```

---

## Documentation index

| Document | Contents |
|---|---|
| [`AGENTS.md`](AGENTS.md) | Rules for contributors and coding agents |
| [`ROADMAP.md`](ROADMAP.md) | Phases, key numbers, progress log |
| [`docs/gaps.md`](docs/gaps.md) | Ordered list of open gaps |
| [`docs/architecture.md`](docs/architecture.md) | Layers and key design decisions |
| [`docs/parity-checklist.md`](docs/parity-checklist.md) | Every Modeler node and its status (generated) |
| [`docs/statistical-parity.md`](docs/statistical-parity.md) | How statistical parity is tested |
| [`docs/models.md`](docs/models.md) | Model and nugget behaviour, score fields |
| [`docs/python-bridge.md`](docs/python-bridge.md) | Python bridge and Python nodes |
| [`docs/oracle.md`](docs/oracle.md) | Recording Modeler's expected answers |
| [`docs/formats/str.md`](docs/formats/str.md) | The `.str` file format, as observed |
| [`docs/gui/DESIGN.md`](docs/gui/DESIGN.md) | UI design, dependencies and licences |
| [`docs/gui/MAC_APP_PLAN.md`](docs/gui/MAC_APP_PLAN.md) | macOS app plan and versions |

Some of these documents are written in Chinese.

---

## Known limitations

- **`dms-cli` on macOS** links unixODBC from Homebrew and needs `brew install unixodbc`; the
  `.app` bundles it.
- **Windows** builds and compiles in CI on macOS, but the app has not yet had a full manual
  pass on Windows.
- **Supernodes** are shown and run, but cannot be edited.
- **Settings forms** cover the most-used nodes; other nodes keep their imported settings and
  show them read-only.
- **Native models are not yet turned into nuggets** after training in the app (C5.0, CHAID,
  ...): that needs a PMML writer. Nuggets imported in `.str` files score normally. Python
  models do generate nuggets.
- **Python Extension nodes from Modeler `.str` files** are kept but usually do not run: the
  way Modeler stores their settings has not been observed, and Modeler's Python extension API
  is Spark-based, which this application does not provide.
- **Not yet implemented:** Streaming TS, GLMM, Cox, Temporal Causal Model; graphics output
  from Extension Output; R syntax in Extension nodes.
- The specific gaps against Modeler's results are listed in [`docs/gaps.md`](docs/gaps.md).

---

## License

Proprietary (`LicenseRef-Proprietary`), for internal team use only. Third-party dependencies
keep their own permissive licences, listed in [`docs/gui/DESIGN.md`](docs/gui/DESIGN.md) §9.

### Licence note: bundled ODBC libraries

The macOS app ships two LGPL libraries as an approved exception to the permissive-only rule
([`docs/gui/DESIGN.md`](docs/gui/DESIGN.md) §9.3): `libodbc` from unixODBC (LGPL-2.1-or-later)
and `libltdl` from GNU libtool (LGPL-2.0-or-later), which `libodbc` needs. They are
unmodified, dynamically linked and kept as separate files in `Contents/Frameworks`, so they
can be replaced. Their licence texts and source locations are in
`Contents/Resources/Licenses/` inside the app.
