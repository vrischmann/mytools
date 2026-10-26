//! `gitjuggling` library root.
//!
//! The binary (`src/main.rs`) is still the M0 clap skeleton and is wired
//! separately; the ported logic modules hang off this lib root so they
//! compile and test on their own (`cargo test -p gitjuggling`) until later
//! milestones hook them into the CLI (RUST_REWRITE_PLAN.md §2, §5).
//!
//! M1 status: `urlnorm` (URL normalization + https→ssh) and `remote`
//! (RemoteRepo/Source types + Forgejo token resolution). `config` and
//! `gitmodules` are wired by their own milestone worker.

pub mod remote;
pub mod urlnorm;
