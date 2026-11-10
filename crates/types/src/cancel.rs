//! Cooperative cancellation shared by jobs, execution and training algorithms.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_a_one_way_flag_and_independent_tokens_do_not() {
        let token = CancelToken::new();
        let clone = token.clone();
        assert_eq!(token.check(), Ok(()));
        std::thread::spawn(move || clone.cancel()).join().unwrap();
        assert_eq!(token.check(), Err(Cancelled));
        token.cancel();
        assert_eq!(Cancelled.to_string(), "cancelled");
        assert!(!CancelToken::default().is_cancelled());
    }
}
