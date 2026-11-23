# Using Data Mining

The desktop app, Python nodes, the command-line tool and the file formats.

## Using the desktop app

The window has a node **palette** on the left, the stream **canvas** in the middle, the **properties** panel on the
right and the **results** panel at the bottom; each can be hidden from the toolbar or the View menu.

**Building a stream.** Drag a node from the palette onto the canvas, or double-click it there (nodes not implemented
yet are greyed out, with the reason as a tooltip). Connect two nodes by dragging from the small circle on a node's right
edge, by holding Option (Alt on Windows) and dragging from the node, or by right-clicking and choosing **Connect From
Here**. Drag on empty canvas to box-select, hold Space and drag to pan, delete a connection by clicking it and pressing
Delete, and rename a node with Return (F2 on Windows).

**Editing settings.** Double-click a node to open its settings in a floating window with **Run** and **Close**; the
properties panel shows the same form. Forms are generated from each node's settings schema (checkboxes, ranges,
choices, field pickers filled from the last run, file pickers, CLEM expressions with syntax checking, tables, code
editors). Every change is one undo step. Nodes whose settings cannot be edited yet show them read-only.

**Running and results.** **Run** (⌘R) runs the whole stream, **Run Selected** (⇧⌘R) the selected node and what it
needs, in the background with a progress bar; **Stop** (⌘.) cancels, including long model training. The results panel
has one tab per terminal node: data tables, output documents (Statistics, Data Audit, Matrix, Means, Analysis, ...),
model summaries, and charts for graph nodes (Histogram, Distribution, Plot, Multiplot, Time Plot, Evaluation,
Collection, t-SNE). A failed node shows why, and nodes that did not run say which upstream node to look at. Streams
often name data files on the machine that saved them: the app looks in the stream's folder first, then in the folders
added with **File > Add Data Folder…**, by file name.

**Saving.** Streams are saved as `.dms` (⌘S, ⇧⌘S); imported `.str` files are never overwritten. Edits are autosaved
and unsaved streams are offered for recovery after a crash.

**Settings** (⌘,): language (System, English, 简体中文), data folders, and the Python executable with a **Check Python**
button.

| Action | macOS | Windows |
|---|---|---|
| New / Open / Close | ⌘N / ⌘O / ⌘W | Ctrl+N / Ctrl+O / Ctrl+W |
| Save / Save As | ⌘S / ⇧⌘S | Ctrl+S / Ctrl+Shift+S |
| Undo / Redo | ⌘Z / ⇧⌘Z | Ctrl+Z / Ctrl+Shift+Z |
| Cut / Copy / Paste / Duplicate | ⌘X / ⌘C / ⌘V / ⌘D | Ctrl+X / Ctrl+C / Ctrl+V / Ctrl+D |
| Run stream / Run selected / Stop | ⌘R / ⇧⌘R / ⌘. | Ctrl+R / Ctrl+Shift+R / Escape |
| Zoom in / out / to fit | ⌘= / ⌘- / ⌘0 | Ctrl+= / Ctrl+- / Ctrl+0 |
| Rename / Delete | ↩ / ⌫ | F2 / Delete |
| Full screen | ⌃⌘F | F11 |

On macOS the menu bar is the native one; on Windows the same menus are drawn in the window. Both are generated from
one command table.

## Python nodes

Python nodes run in **your own Python**, as a separate process: no Python is bundled and libpython is not linked. Data
goes to Python and back as Apache Arrow files in a private temporary folder; scripts see a pandas DataFrame.

```sh
python3 -m pip install pyarrow pandas            # every Python node
python3 -m pip install scikit-learn              # Random Forest, GMM, One-Class SVM, KDE, t-SNE
python3 -m pip install xgboost                   # XGBoost Tree / Linear
python3 -m pip install hdbscan                   # HDBSCAN
python3 -m pip install imbalanced-learn          # SMOTE
brew install libomp                              # macOS only: OpenMP runtime xgboost needs
```

The app looks for Python in this order: the path in Settings; `DMS_PYTHON`; on macOS `/opt/homebrew/bin/python3` and
`/usr/local/bin/python3` (apps opened from Finder get a short `PATH`); then `python3` and `python` (`py -3` first on
Windows). A configured path that does not work is reported, never silently replaced, and missing packages produce the
exact install command.

**Extension scripts** see `modelerData` (the input as a pandas DataFrame, `None` in Extension Import),
`modelerDataModel` (a list of `{name, storage, measurement, role}`) and `print(...)` (the Extension Output result).
Import and Transform scripts leave their result in `modelerData`; errors report the line of your script.

| Palette | Node | Library |
|---|---|---|
| Sources / Record Ops | Extension Import, Extension Transform | your script |
| Record Ops | SMOTE | imbalanced-learn |
| Graphs | t-SNE | scikit-learn |
| Modeling | Random Forest, Gaussian Mixture, One-Class SVM, KDE | scikit-learn |
| Modeling | XGBoost Tree, XGBoost Linear | xgboost |
| Modeling | HDBSCAN | hdbscan |
| Modeling | Extension Model | your build and score scripts |
| Output / Export | Extension Output, KDE Simulation, Extension Export | your script / scikit-learn |

As in Modeler, running a Python build node trains the model and places a **model nugget** next to it. The model is
stored inside the stream as a pickle attachment, so it survives saving and reopening. A continuous target is regressed,
any other is classified; with a partition field only the training partition is used.

> **Security.** Running a stream runs the Python code in it, and loading a saved Python model unpickles it, which can
> also run code, with your permissions. This is the same trust model as Modeler: only run streams you trust.

## Command-line tool

`dms-cli` works without the window (on macOS it ships inside the app; from source use `cargo run -p dms-cli -- <command>`):

```text
dms-cli xml <file.str>                 print the decoded stream document as XML
dms-cli inspect <file.str>             print nodes, links and attachments as JSON
dms-cli coverage [--json] <path>...    report which catalog nodes the .str files use
dms-cli convert <in.str> <out.dms>     import a stream and save it in the native format
dms-cli run <file.str|file.dms> --out <dir> [--data-dir <dir>]... [--terminal-order <uid,...>]
                                       run the stream; write the data reaching each terminal
                                       node to <dir>/<uid>.csv and a report to <dir>/run.json
```

Export nodes write under `<dir>/exports/`, never over the paths saved in the stream.

## File formats

| Format | Read | Write | Notes |
|---|---|---|---|
| Modeler stream `.str` | ✓ | — | ZIP + Fast Infoset (ITU-T X.891); unknown nodes and settings preserved; node settings as observed in `docs/formats/` |
| Native stream `.dms` | ✓ | ✓ | TOML text; attachments (models, extension settings) as base64; saved atomically |
| Delimited text, fixed-width | ✓ | ✓ | Var. File, Fixed File, Flat File export |
| SPSS Statistics `.sav` / `.zsav` | ✓ | ✓ | Our own reader and writer |
| Excel `.xlsx`, SAS, JSON, XML | ✓ | ✓ | |
| Databases (ODBC) | ✓ | ✓ | Needs an ODBC driver manager |
| PMML 4.3 | ✓ | — | Model nuggets in `.str` files score from their saved PMML |
