//! Node kinds the engine executes.
//!
//! Parity is measured against this list, so it must name only node kinds with a working
//! implementation and tests. It becomes the real node registry once execution lands.

/// Catalog ids (`catalog::NodeSpec::id`) of implemented node kinds.
pub const IMPLEMENTED: &[&str] = &[];

/// Whether the node kind `id` is implemented.
pub fn is_implemented(id: &str) -> bool {
    IMPLEMENTED.contains(&id)
}
