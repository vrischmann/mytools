//! Bubbletea-style inline rendering: render the same ratatui widget tree
//! through a [`TestBackend`] buffer, then manage the scrollback ourselves
//! (cursor-up + clear + print) instead of ratatui's `Viewport::Inline`,
//! which anchors by querying the cursor position (DSR) and fails on any
//! terminal or pty that does not answer — bubbletea never queries, and the
//! port must match that robustness. Decision A is unchanged: the widgets
//! are ratatui's, only the inline transport is hand-managed.

use std::io::{self, Write};

use ratatui::backend::TestBackend;
use ratatui::buffer::Cell;
use ratatui::style::{Color, Modifier};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

/// Draws styled [`Line`]s inline in the normal terminal buffer, erasing the
/// previous frame first. The final frame stays in scrollback after exit,
/// exactly like bubbletea's inline mode.
pub struct Inline {
    prev_height: usize,
}

impl Inline {
    pub fn new() -> Inline {
        Inline { prev_height: 0 }
    }

    /// Renders one frame at the terminal's current size. Only rows up to
    /// the last non-blank one are written, so scrollback is not padded
    /// with empty lines (bubbletea behavior).
    pub fn draw(&mut self, lines: &[Line<'static>]) -> anyhow::Result<()> {
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| {
            frame.render_widget(Paragraph::new(lines.to_vec()), frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let row_is_blank = |y: u16| -> bool {
            (0..width).all(|x| {
                let cell = buffer.cell((x, y)).expect("cell in buffer");
                cell.symbol() == " " && cell.fg == Color::Reset
            })
        };
        let mut last_row = 0usize;
        for y in (0..height as usize).rev() {
            if !row_is_blank(y as u16) {
                last_row = y;
                break;
            }
        }

        let mut out = io::stdout();
        if self.prev_height > 0 {
            // Move to the top of the previous frame and clear it. The
            // cursor is already at column 0: every frame ends with \r\n,
            // and cursor-up preserves the column.
            write!(out, "\x1b[{}A\x1b[J", self.prev_height)?;
        }
        let mut frame_height = 0usize;
        for y in 0..=last_row {
            let row: Vec<&Cell> = (0..width)
                .map(|x| buffer.cell((x, y as u16)).expect("cell in buffer"))
                .collect();
            let styled = styled_row(&row);
            // Trailing padding would push the row against the right margin,
            // where a full-width line wraps before the line break and costs
            // a blank line per frame. Trim it; it carries no information.
            let styled = styled.trim_end().to_owned();
            // Raw mode disables the \n -> \r\n translation (ONLCR), so
            // the break must be explicit: \n alone leaves the cursor in
            // the same column and the frame "streams" instead of redrawing.
            write!(out, "{styled}\r\n")?;
            frame_height += 1;
        }
        out.flush()?;
        self.prev_height = frame_height;
        Ok(())
    }
}

/// Serializes one buffer row back to ANSI text (symbol + style changes),
/// preserving the machine's palette.
fn styled_row(row: &[&Cell]) -> String {
    let mut out = String::new();
    let mut current: Option<(Color, Color, Modifier)> = None;
    for cell in row {
        let style = (cell.fg, cell.bg, cell.modifier);
        if current != Some(style) {
            out.push_str("\x1b[0m");
            out.push_str(&fg_sgr(cell.fg));
            out.push_str(&bg_sgr(cell.bg));
            if cell.modifier.contains(Modifier::BOLD) {
                out.push_str("\x1b[1m");
            }
            current = Some(style);
        }
        out.push_str(cell.symbol());
    }
    if current.is_some() {
        out.push_str("\x1b[0m");
    }
    out
}

fn fg_sgr(color: Color) -> String {
    match color {
        Color::Reset => String::new(),
        Color::Indexed(n) => format!("\x1b[38;5;{n}m"),
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
        other => {
            let n = ansi_index(other);
            format!("\x1b[38;5;{n}m")
        }
    }
}

fn bg_sgr(color: Color) -> String {
    match color {
        Color::Reset => String::new(),
        Color::Indexed(n) => format!("\x1b[48;5;{n}m"),
        Color::Rgb(r, g, b) => format!("\x1b[48;2;{r};{g};{b}m"),
        other => {
            let n = ansi_index(other);
            format!("\x1b[48;5;{n}m")
        }
    }
}

fn ansi_index(color: Color) -> u8 {
    match color {
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::White => 7,
        Color::Gray => 8,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightBlue => 12,
        Color::LightYellow => 11,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        _ => 7,
    }
}
