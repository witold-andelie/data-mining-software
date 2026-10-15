# dms-sav

IBM SPSS Statistics system files (`.sav`, bytecode-compressed data, ZLIB `.zsav`): dictionary and
case data reader. Layer L0, standalone.

- Spec source (documentation only): GNU PSPP Developers Guide, chapter "System File Format",
  <https://www.gnu.org/software/pspp/pspp-dev/html_node/System-File-Format.html> and its
  sub-pages. Fetched 2026-10-15 (sub-pages after a gnu.org outage via the web.archive.org copy of
  the same pages); local copies in `D:\spss-modeler\rea-work\specs\sav\`.
- Clean room: no source code of PSPP, ReadStat, pyreadstat, pandas, haven or any other reader
  was read.

## Supported

Header (both byte orders, detected by the layout code), variable records (numeric, short/long
string with continuation records, formats, labels, missing values including ranges), value
labels (records 3+4), document record (6), extension subtypes 3, 4, 11, 13, 14, 16, 20, 21, 22,
dictionary termination (999), data: uncompressed, bytecode (bias from header) and ZLIB
(`$FL3`: header, blocks, trailer). Other extension records are kept raw in
`SavFile::unknown_extensions`; malformed known extensions are kept raw too and reported in
`SavFile::warnings`.

Strings use the encoding named by subtype 20, else the character code of subtype 3, else
windows-1252, and have trailing spaces/NULs removed. Numbers, dates and user-missing values stay
raw: `Value::SystemMissing` is distinct, user-missing values are exposed through
`Variable::missing`, and `Format::is_date_time` tells date/time formats (seconds since
1582-10-14).

## Not supported

EBCDIC files, IBM/VAX floating point (reported as `Unsupported`), subtype 7/8 (multiple
response sets), 10, 17, 18, 19 (kept raw), the ReadStat duplicate-label quirk.

## Never crash

All reads are bounds-checked and return `Error` (offset + kind). `Limits` caps variable records,
materialised cells and decompressed size; row storage is reserved only up to what the data can
hold. Tests include truncation and bit-flip mutation, and a `corpus` feature test (also
comparing values with Modeler CSV exports under `oracle/probes/sav_to_csv/`).
