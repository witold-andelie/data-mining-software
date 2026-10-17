//! Which catalog nodes are implemented: derived from the executor's registry, so parity never
//! drifts from the code that actually runs.

use dms_catalog::{Coverage, Scope};

pub fn is_implemented(catalog_id: &str) -> bool {
    dms_exec::registry::is_implemented(catalog_id)
}

pub fn coverage(scope: Scope) -> Coverage {
    dms_catalog::coverage(scope, is_implemented)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Implemented MVP nodes; raise it when nodes land, never lower it.
    const MVP_FLOOR: usize = 49;

    #[test]
    fn implemented_ids_exist_in_catalog() {
        for id in dms_exec::registry::implemented() {
            assert!(
                dms_catalog::NODES.iter().any(|spec| spec.id == id),
                "unknown node {id}"
            );
        }
    }

    #[test]
    fn mvp_parity_never_drops() {
        assert!(coverage(Scope::Mvp).implemented >= MVP_FLOOR);
    }
}
