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
    /// Content of the previously drawn frame, for line-level diffing.
    prev_lines: Vec<String>,
    /// Rows the cursor sits below the frame anchor after the last draw
    /// (grows with the frame, never shrinks below what was painted).
    prev_rows: usize,
}

impl Inline {
    pub fn new() -> Inline {
        Inline {
            prev_lines: Vec::new(),
            prev_rows: 0,
        }
    }

    /// Renders one frame at the terminal's current size. Frames are
    /// diffed line by line, bubbletea-style: only changed lines are
    /// rewritten (with a per-line clear), so an unchanged majority is
    /// never blanked — the whole-frame erase of the first cut caused a
    /// visible flicker on every spinner tick.
    pub fn draw(&mut self, lines: &[Line<'static>]) -> anyhow::Result<()> {
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| {
            frame.render_widget(Paragraph::new(lines.to_vec()), frame.area());
        })?;
        let buffer = terminal.backend().buffer().clone();

        let mut last_row = 0usize;
        for y in (0..height as usize).rev() {
            let blank = (0..width).all(|x| {
                let cell = buffer.cell((x, y as u16)).expect("cell in buffer");
                cell.symbol() == " " && cell.fg == Color::Reset
            });
            if !blank {
                last_row = y;
                break;
            }
        }

        let new_lines: Vec<String> = (0..=last_row)
            .map(|y| {
                let row: Vec<&Cell> = (0..width)
                    .map(|x| buffer.cell((x, y as u16)).expect("cell in buffer"))
                    .collect();
                styled_row(&row).trim_end().to_owned()
            })
            .collect();

        // Identical frame: emit nothing, like bubbletea's renderer —
        // no cursor churn, no repaint, no flicker window at all.
        if new_lines == self.prev_lines {
            return Ok(());
        }

        let mut out = io::stdout();
        if self.prev_rows > 0 {
            // The cursor sits prev_rows below the anchor; return to it.
            write!(out, "\x1b[{}A", self.prev_rows)?;
        }
        // The frame box keeps its high-water mark: a shrinking frame still
        // repaints (blanks) the rows it no longer uses, so the cursor
        // arithmetic stays consistent.
        let count = self.prev_rows.max(new_lines.len());
        for i in 0..count {
            let changed = i >= new_lines.len()
                || i >= self.prev_lines.len()
                || new_lines[i] != self.prev_lines[i];
            if changed {
                // Erase exactly this line and rewrite it — one burst, no
                // whole-frame blank window.
                write!(out, "\r\x1b[K")?;
                if i < new_lines.len() {
                    write!(out, "{}", new_lines[i])?;
                }
            }
            // Raw mode needs the explicit carriage return (ONLCR is off).
            write!(out, "\r\n")?;
        }
        out.flush()?;
        self.prev_rows = count;
        self.prev_lines = new_lines;
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
