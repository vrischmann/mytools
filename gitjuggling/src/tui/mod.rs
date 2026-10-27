//! The sync TUI (Go: `internal/tui`), per decisions A and H:
//!
//! - [`sync`]: the pure `SyncApp` state machine — no terminal, no threads;
//!   the ported `tui/sync_test.go` cases run headless against it.
//! - [`app`]: the thin runtime — one blocking loop over one merged channel
//!   (keys, worker results, ticks, resize), inline viewport.
//! - [`view`]/[`styles`]/[`widgets`]: idiomatic ratatui rendering; the two
//!   bubbles widgets (spinner, plan viewport) are written, not picked.

mod app;
pub mod styles;
mod sync;
mod view;
mod widgets;

pub use app::run_sync;
pub(crate) use sync::SyncApp;
pub(crate) use widgets::PlanView;
