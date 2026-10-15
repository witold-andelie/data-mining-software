//! IBM SPSS Modeler `.str` stream files (L0, standalone).
//!
//! A `.str` file is a ZIP archive containing the stream as a fast infoset document (ITU-T X.891,
//! a binary encoding of XML) plus attachments under `data/`. This crate owns that format: the
//! container ([`container`]) and the fast infoset decoder ([`fastinfoset`]). Interpreting the
//! decoded XML as a stream (nodes, links, properties) belongs to the stream model layer.
//!
//! The format facts were established by differential observation of files saved by Modeler
//! (`cargo xtask probe str_basics`) and by the public X.891 specification; see
//! `docs/formats/str.md`.

pub mod container;
pub mod fastinfoset;

pub use container::{
    ArchiveError, ArchiveLimits, Attachment, AttachmentKind, StreamArchive, read_archive,
};
