//! The TUI palette (Go: `tui/styles.go`). Idiomatic ratatui styles replace
//! lipgloss render-to-string calls (decision A); the ANSI-256 indices are
//! kept verbatim so the colors match the Go TUI.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Labels/headers (Go `LabelStyle`, ANSI 12 blue).
pub fn label() -> Style {
    Style::new().fg(Color::Indexed(12))
}

/// Success messages and checkmarks (Go `SuccessStyle`, ANSI 10 green).
pub fn success() -> Style {
    Style::new().fg(Color::Indexed(10))
}

/// Error messages and cross marks (Go `ErrorStyle`, ANSI 9 red).
pub fn error() -> Style {
    Style::new().fg(Color::Indexed(9))
}

/// Section titles (Go `TitleStyle`, ANSI 14 cyan).
pub fn title() -> Style {
    Style::new().fg(Color::Indexed(14))
}

/// Moves and warnings (Go `WarnStyle`, ANSI 11 yellow).
pub fn warn() -> Style {
    Style::new().fg(Color::Indexed(11))
}

/// Dimmed/secondary info (Go `DimStyle`, ANSI 8 gray).
pub fn dim() -> Style {
    Style::new().fg(Color::Indexed(8))
}

/// Emphasis (Go `BoldStyle`).
pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

/// Section `===` delimiters (Go `SectionStyle`, bright white + bold).
fn section() -> Style {
    Style::new()
        .fg(Color::Indexed(15))
        .add_modifier(Modifier::BOLD)
}

/// Git stdout (Go `StdoutStyle`, #b0b0b0).
pub fn stdout() -> Style {
    Style::new().fg(Color::Rgb(0xb0, 0xb0, 0xb0))
}

/// Git stderr (Go `StderrStyle`, #db9a9a).
pub fn stderr() -> Style {
    Style::new().fg(Color::Rgb(0xdb, 0x9a, 0x9a))
}

/// Go `SectionHeader` — `=== Title ===` with the delimiters and the title
/// styled separately.
pub fn section_header(heading: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("=== ", section()),
        Span::styled(heading.to_owned(), title().add_modifier(Modifier::BOLD)),
        Span::styled(" ===", section()),
    ])
}

/// Go `Checkmark` — a green ✓.
pub fn checkmark() -> Span<'static> {
    Span::styled("✓".to_owned(), success())
}

/// Go `CrossMark` — a red ✗.
pub fn cross_mark() -> Span<'static> {
    Span::styled("✗".to_owned(), error())
}

/// Go `Arrow` — a blue →.
pub fn arrow() -> Span<'static> {
    Span::styled("→".to_owned(), label())
}

/// Go `Bullet` — a dimmed ●.
pub fn bullet() -> Span<'static> {
    Span::styled("●".to_owned(), dim())
}

/// Go `PendingBullet` — a dimmed ○.
pub fn pending_bullet() -> Span<'static> {
    Span::styled("○".to_owned(), dim())
}

/// Go `CountDisplay` — "12 to update, 3 to move" with colored counts.
pub fn count_display(updates: usize, moves: usize, clones: usize) -> Vec<Span<'static>> {
    let mut parts: Vec<Vec<Span<'static>>> = Vec::new();
    if updates > 0 {
        parts.push(vec![
            Span::styled(updates.to_string(), success()),
            Span::styled(" to update".to_owned(), label()),
        ]);
    }
    if moves > 0 {
        parts.push(vec![
            Span::styled(moves.to_string(), warn()),
            Span::styled(" to move".to_owned(), label()),
        ]);
    }
    if clones > 0 {
        parts.push(vec![
            Span::styled(clones.to_string(), title()),
            Span::styled(" to clone".to_owned(), label()),
        ]);
    }
    let mut spans = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(", "));
        }
        spans.extend(part);
    }
    spans
}
