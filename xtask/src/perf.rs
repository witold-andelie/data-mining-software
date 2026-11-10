//! Headless CPU frame budget; no display, window, or GPU required.

use crate::{Result, cargo};

pub fn run(args: &[String]) -> Result<()> {
    if !args.is_empty() {
        return Err("usage: cargo xtask perf".into());
    }
    println!("CPU frame budget: 200 nodes p95 <= 4.00 ms; 1000 nodes reported for comparison.");
    println!(
        "dms --bench requires a real window/display and GPU; unavailable in sandbox/CI, not run here."
    );
    cargo(&[
        "test",
        "--release",
        "-p",
        "dms-ui",
        "--test",
        "perf",
        "--",
        "--ignored",
        "--nocapture",
    ])?;
    println!("xtask perf: CPU frame budget passed (GPU/window performance not measured)");
    Ok(())
}
