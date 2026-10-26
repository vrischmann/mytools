//! Test-only helpers (port target of Go `internal/gittest`).
//!
//! Compiled only under `cfg(test)`: the fixture harness must never ship in the
//! binary and, like Go's `internal/gittest`, is not reachable from outside the
//! crate's own tests. If a later milestone needs it from `tests/` integration
//! tests, move the body to `tests/common/mod.rs` (RUST_REWRITE_PLAN.md §2).

pub mod fixtures;
