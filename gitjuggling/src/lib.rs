//! `gitjuggling` library root.
//!
//! The binary (`src/main.rs`) is still the M0 clap skeleton and is wired
//! separately; the ported logic modules hang off this lib root so they
//! compile and test on their own (`cargo test -p gitjuggling`) until later
//! milestones hook them into the CLI (RUST_REWRITE_PLAN.md §2, §5).
//!
//! M1: `urlnorm` (URL normalization + https→ssh), `remote`
//! (RemoteRepo/Source types + Forgejo token resolution), `config`
//! (config types, YAML load, validate, IsIgnored, ported filepath.Match
//! ignore matcher, pattern-rule template parser/expander), and
//! `gitmodules` (the .gitmodules parser). Later milestones wire these
//! into the CLI.

pub mod config;
pub mod git;
pub mod gitmodules;
pub mod remote;
#[cfg(test)]
pub mod testing;
pub mod urlnorm;
