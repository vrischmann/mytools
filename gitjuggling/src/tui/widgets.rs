//! The two widgets the plan says must be **written**, not picked (decision A):
//! a tick-driven spinner for the loading phase, and a scrollable plan view
//! with bubbles' viewport keymap (pgup/pgdn, b/f, u/d, j/k, g/G).

use ratatui::text::Line;

use super::sync::Key;

/// A spinner tick (Go: `spinner.TickMsg`) — drives frame advancement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpinnerTick;

/// The bubbles `spinner.Dot` spinner, tick-driven (Go: `spinner.Model`).
///
/// Frames and 100ms cadence match the Charm spinner defaults; the sync
/// model styles it green (`SuccessStyle` in Go's `NewSyncModel`).
#[derive(Debug, Clone, Default)]
pub struct Spinner {
    frame: usize,
}

impl Spinner {
    pub fn new() -> Spinner {
        Spinner { frame: 0 }
    }

    /// Advance one frame (Go: `m.spinner.Update(TickMsg)`).
    pub fn advance(&mut self) {
        self.frame = (self.frame + 1) % FRAMES.len();
    }

    /// The current frame's glyph.
    pub fn frame(&self) -> &'static str {
        FRAMES[self.frame]
    }
}

const FRAMES: [&str; 8] = ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"];

/// The spinner's tick interval (bubbles `spinner.Dot` FPS 10).
pub const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// A scrollable view over styled lines with the bubbles `viewport.Model`
/// keymap (Go: `viewport.Model`; §7 decision A — written, not picked).
#[derive(Debug, Clone, Default)]
pub struct PlanView {
    lines: Vec<Line<'static>>,
    offset: usize,
    width: usize,
    height: usize,
}

impl PlanView {
    pub fn new() -> PlanView {
        PlanView::default()
    }

    pub fn set_content(&mut self, lines: Vec<Line<'static>>) {
        self.lines = lines;
        self.clamp();
    }

    /// Go: the `WindowSizeMsg` handler resizing the viewport to
    /// `height - 2` lines.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.clamp();
    }

    /// The bubbles viewport default keymap: k/up and j/down move a line,
    /// b/pgup and f/pgdn a page, u/d half a page, g/home and G/end the
    /// extremes.
    pub fn handle_key(&mut self, key: Key) {
        match key {
            Key::Up | Key::Char('k') => self.scroll_up(1),
            Key::Down | Key::Char('j') => self.scroll_down(1),
            Key::PgUp | Key::Char('b') => self.scroll_up(self.height),
            Key::PgDown | Key::Char('f') => self.scroll_down(self.height),
            Key::Char('u') => self.scroll_up(self.height.div_ceil(2)),
            Key::Char('d') => self.scroll_down(self.height.div_ceil(2)),
            Key::Home | Key::Char('g') => self.offset = 0,
            Key::End | Key::Char('G') => {
                self.offset = self.max_offset();
            }
            _ => {}
        }
    }

    fn scroll_up(&mut self, n: usize) {
        self.offset = self.offset.saturating_sub(n);
    }

    fn scroll_down(&mut self, n: usize) {
        self.offset = (self.offset + n).min(self.max_offset());
    }

    fn max_offset(&self) -> usize {
        self.lines.len().saturating_sub(self.height)
    }

    fn clamp(&mut self) {
        self.offset = self.offset.min(self.max_offset());
    }

    /// The visible window of the content (Go: `viewport.View()`).
    pub fn visible(&self) -> &[Line<'static>] {
        let end = (self.offset + self.height).min(self.lines.len());
        &self.lines[self.offset..end]
    }

    /// Test-only for now; M5b's executing view may expose a scroll
    /// indicator.
    #[cfg(test)]
    pub fn offset(&self) -> usize {
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Span;

    fn lines(n: usize) -> Vec<Line<'static>> {
        (0..n)
            .map(|i| {
                Line::from(Span::styled(
                    format!("line {i}"),
                    Style::default()
                        .fg(Color::Indexed(8))
                        .add_modifier(Modifier::BOLD),
                ))
            })
            .collect()
    }

    #[test]
    fn spinner_cycles_the_bubbles_dot_frames() {
        let mut s = Spinner::new();
        assert_eq!(s.frame(), "⣾");
        for (i, want) in FRAMES.iter().enumerate().skip(1) {
            s.advance();
            assert_eq!(s.frame(), *want, "frame {i}");
        }
        s.advance();
        assert_eq!(s.frame(), FRAMES[0], "wraps");
    }

    #[test]
    fn plan_view_scrolls_with_the_bubbles_keymap() {
        let mut v = PlanView::new();
        v.resize(80, 10);
        v.set_content(lines(50));
        assert_eq!(v.offset(), 0);

        v.handle_key(Key::Down);
        assert_eq!(v.offset(), 1);
        v.handle_key(Key::Char('j'));
        assert_eq!(v.offset(), 2);
        v.handle_key(Key::PgDown);
        assert_eq!(v.offset(), 12);
        v.handle_key(Key::Char('f'));
        assert_eq!(v.offset(), 22);
        v.handle_key(Key::Char('d'));
        assert_eq!(v.offset(), 27, "half page down");
        v.handle_key(Key::PgUp);
        assert_eq!(v.offset(), 17);
        v.handle_key(Key::Char('u'));
        assert_eq!(v.offset(), 12, "half page up");
        v.handle_key(Key::End);
        assert_eq!(v.offset(), 40, "bottom = len - height");
        v.handle_key(Key::Home);
        assert_eq!(v.offset(), 0);
        v.handle_key(Key::Up);
        assert_eq!(v.offset(), 0, "clamped at top");
    }

    #[test]
    fn plan_view_visible_window_follows_the_offset() {
        let mut v = PlanView::new();
        v.resize(80, 3);
        v.set_content(lines(10));
        v.handle_key(Key::Char('j'));
        let visible = v.visible();
        assert_eq!(visible.len(), 3);
        assert_eq!(visible[0].to_string(), "line 1");
    }
}
