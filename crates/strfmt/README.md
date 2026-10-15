# dms-strfmt

IBM SPSS Modeler `.str` stream files: the ZIP container and a Fast Infoset decoder.

- Fast Infoset: implemented from **ITU-T Rec. X.891 (05/2005) | ISO/IEC 24824-1:2007** — Annex C
  (bit-level encodings), clause 7 (vocabulary table indexes), clause 9 (built-in restricted
  alphabets), clause 10 (built-in encoding algorithms). External vocabularies and user-defined
  encoding algorithms are reported as unsupported.
- `.str` container facts come from differential observation of files saved by Modeler 19.0.0.0.26
  (`cargo xtask probe str_basics`); see `docs/formats/str.md`.
- No Modeler code was read, decompiled or copied.

Never-crash properties: all errors are `Result`s; element nesting is decoded and dropped without
recursion; vocabulary references share strings (`Arc<str>`), so the tree is proportional to the
input; archive members and totals are size-limited. Mutation tests cover truncation and bit flips.
