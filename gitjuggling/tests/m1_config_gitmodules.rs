//! M1a test shim (RUST_REWRITE_PLAN.md §5, phase M1).
//!
//! `src/config.rs` and `src/gitmodules.rs` are not yet wired into the binary
//! (main.rs gains `mod config;` / `mod gitmodules;` when the first real
//! handler lands). Until then, this integration test compiles them as modules
//! of the test crate — which sets `cfg(test)` for the whole module tree — so
//! their `#[cfg(test)] mod tests` units run under `cargo test -p gitjuggling`.
//!
//! Delete this file once main.rs declares the modules.

#[path = "../src/config.rs"]
mod config;

#[path = "../src/gitmodules.rs"]
mod gitmodules;
