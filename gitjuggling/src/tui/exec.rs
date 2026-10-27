//! The `exec` progress TUI (Go: `tui/exec.go`) — decision A: a flat bar
//! (no bubbles gradient; accepted §8 deviation) and the same three phases:
//! discovering, running, done. The summary always shows failures, shows
//! full output only under `--verbose`, and ends with the counts and
//! elapsed time.

use std::path::PathBuf;
use std::time::Instant;

use super::styles;
use super::widgets::{Spinner, TICK_INTERVAL};
use ratatui::style::Stylize;
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecPhase {
    Discovering,
    Running,
    Done,
}

/// One repo's command outcome (Go: `tui.ExecResultMsg`). stdout and stderr
/// are the combined output split only by the exit status, exactly like Go's
/// `CombinedOutput` fields.
#[derive(Debug, Clone)]
pub struct ExecResult {
    pub path: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub enum ExecMsg {
    Discover(Vec<PathBuf>),
    Result(ExecResult),
    Tick,
}

/// Go: `ExecModel`.
pub struct ExecApp {
    phase: ExecPhase,
    pub(crate) verbose: bool,
    git_args: Vec<String>,
    total: usize,
    completed: usize,
    spinner: Spinner,
    results: Vec<ExecResult>,
    start: Option<Instant>,
    elapsed_secs: f64,
}

impl ExecApp {
    pub fn new(git_args: Vec<String>, verbose: bool) -> ExecApp {
        ExecApp {
            phase: ExecPhase::Discovering,
            verbose,
            git_args,
            total: 0,
            completed: 0,
            spinner: Spinner::new(),
            results: Vec::new(),
            start: None,
            elapsed_secs: 0.0,
        }
    }

    pub fn handle_msg(&mut self, msg: ExecMsg) -> bool {
        // Returns true when the loop must exit.
        match msg {
            ExecMsg::Tick => {
                self.spinner.advance();
                false
            }
            ExecMsg::Discover(paths) => {
                self.phase = ExecPhase::Running;
                self.total = paths.len();
                self.start = Some(Instant::now());
                false
            }
            ExecMsg::Result(result) => {
                self.completed += 1;
                self.results.push(result);
                if self.completed >= self.total {
                    self.phase = ExecPhase::Done;
                    self.elapsed_secs =
                        self.start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
                    return true;
                }
                false
            }
        }
    }

    /// Go: `HasFailures` — drives the process exit code.
    pub fn has_failures(&self) -> bool {
        self.results.iter().any(|r| !r.success)
    }

    fn repo_name(path: &str) -> &str {
        path.rsplit('/').next().unwrap_or(path)
    }

    fn progress_line(&self) -> Line<'static> {
        let latest = self.results.last();
        let label = match latest {
            Some(r) => format!(
                "  {}/{}  {}",
                self.completed,
                self.total,
                Self::repo_name(&r.path)
            ),
            None => format!("  {}/{}", self.completed, self.total),
        };
        let width = 40usize;
        let filled = (self.completed * width) / self.total.max(1);
        Line::from(vec![
            Span::raw("  "),
            Span::styled("█".repeat(filled), styles::success()),
            Span::styled("░".repeat(width - filled), styles::dim()),
            Span::raw(label),
        ])
    }

    /// Go: `View()`.
    pub fn render(&self) -> Vec<Line<'static>> {
        match self.phase {
            ExecPhase::Discovering => vec![Line::from(format!(
                "  {} Scanning repos...",
                self.spinner.frame()
            ))],
            ExecPhase::Running => {
                let mut lines = vec![Line::from(format!(
                    "  Running git {}",
                    self.git_args.join(" ")
                ))];
                lines.push(self.progress_line());
                lines
            }
            ExecPhase::Done => self.render_summary(),
        }
    }

    /// Go: `renderSummary` — failures always, output only when verbose.
    fn render_summary(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let succeeded: Vec<&ExecResult> = self.results.iter().filter(|r| r.success).collect();
        let failed: Vec<&ExecResult> = self.results.iter().filter(|r| !r.success).collect();

        if self.verbose && !succeeded.is_empty() {
            lines.push(styles::section_header("Output"));
            lines.push(Line::from(""));
            for item in &succeeded {
                lines.push(Line::from(Span::styled(
                    item.path.clone(),
                    styles::success(),
                )));
                if !item.stdout.is_empty() {
                    lines.push(Line::from(Span::styled(
                        item.stdout.clone(),
                        styles::stdout(),
                    )));
                }
                if !item.stderr.is_empty() {
                    lines.push(Line::from(Span::styled(
                        item.stderr.clone(),
                        styles::stderr(),
                    )));
                }
                lines.push(Line::from(""));
            }
        }

        if !failed.is_empty() {
            if !self.verbose {
                lines.push(Line::from(""));
            }
            lines.push(styles::section_header("Failed Items"));
            lines.push(Line::from(""));
            for item in &failed {
                lines.push(Line::from(Span::styled(item.path.clone(), styles::error())));
                if !item.stdout.is_empty() {
                    lines.push(Line::from(Span::styled(
                        item.stdout.clone(),
                        styles::stdout(),
                    )));
                }
                if !item.stderr.is_empty() {
                    lines.push(Line::from(Span::styled(
                        item.stderr.clone(),
                        styles::stderr(),
                    )));
                }
                lines.push(Line::from(""));
            }
        }

        lines.push(styles::section_header("Summary"));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Succeeded:", styles::label()),
            Span::raw(" "),
            Span::styled(succeeded.len().to_string(), styles::success()),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Failed:   ", styles::label()),
            Span::raw(" "),
            Span::styled(failed.len().to_string(), styles::error()),
        ]));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Time:     ", styles::label()),
            Span::raw(" "),
            Span::raw(format!("{:.2}s", self.elapsed_secs)).bold(),
        ]));
        lines
    }
}

enum Event {
    Key(super::sync::Key),
    Msg(ExecMsg),
}

/// The exec runtime: same merged-channel pattern as the sync loop — one
/// key-reader thread, one ticker, the repo scan and the per-repo git
/// commands on worker threads.
pub fn run_exec(app: &mut ExecApp, depth: usize, concurrency: usize) -> anyhow::Result<bool> {
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let viewport_height = height.min(40);
    let _ = (width, viewport_height);

    crossterm::terminal::enable_raw_mode()?;
    let mut inline = super::inline::Inline::new();

    let result = (|| -> anyhow::Result<bool> {
        let (tx, rx) = std::sync::mpsc::channel::<Event>();

        // Key reader (q quits).
        {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("exec-keys".into())
                .spawn(move || {
                    loop {
                        match crossterm::event::read() {
                            Ok(crossterm::event::Event::Key(key)) => {
                                if key.kind == crossterm::event::KeyEventKind::Press
                                    && tx
                                        .send(Event::Key(convert_exec_key(key.code, key.modifiers)))
                                        .is_err()
                                {
                                    break;
                                }
                            }
                            Ok(crossterm::event::Event::Resize(w, h)) => {
                                let _ = tx.send(Event::Msg(ExecMsg::Discover(Vec::new())));
                                let _ = (w, h);
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                })?;
        }
        // Ticker.
        {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("exec-tick".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(TICK_INTERVAL);
                        if tx.send(Event::Msg(ExecMsg::Tick)).is_err() {
                            break;
                        }
                    }
                })?;
        }
        // Discovery: the decision-E walk (nearest .gitmodules, .git files
        // visible, depth-limited) on a worker thread.
        {
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("exec-discover".into())
                .spawn(move || {
                    let paths = crate::discover::exec_repository_paths(depth);
                    let _ = tx.send(Event::Msg(ExecMsg::Discover(paths)));
                })?;
        }

        let _ = width;
        while let Ok(event) = rx.recv() {
            let exit = match event {
                Event::Key(super::sync::Key::Char('q'))
                | Event::Key(super::sync::Key::CtrlChar('c')) => true,
                Event::Key(_) => false,
                Event::Msg(ExecMsg::Discover(paths)) => {
                    let count = paths.len();
                    spawn_repo_runners(tx.clone(), paths, &app.git_args, concurrency);
                    app.handle_msg(ExecMsg::Discover(vec![PathBuf::new(); count]));
                    false
                }
                Event::Msg(msg) => app.handle_msg(msg),
            };
            inline.draw(&app.render())?;
            if exit {
                break;
            }
        }
        Ok(false)
    })();

    crossterm::terminal::disable_raw_mode()?;
    result.map(|_| app.has_failures())
}

/// Runs `git <args>` in every discovered repo with bounded parallelism
/// (Go: the goroutine fan-out in cmd/exec.go with its semaphore). Output is
/// combined; the exit status decides which field carries it.
fn spawn_repo_runners(
    tx: std::sync::mpsc::Sender<Event>,
    paths: Vec<PathBuf>,
    git_args: &[String],
    concurrency: usize,
) {
    let git_args: Vec<String> = git_args.to_vec();
    let workers = concurrency.max(1).min(paths.len().max(1));
    let chunk = paths.len().div_ceil(workers);
    let _ = std::thread::Builder::new()
        .name("exec-run".into())
        .spawn(move || {
            std::thread::scope(|scope| {
                for chunk_paths in paths.chunks(chunk) {
                    let tx = tx.clone();
                    let git_args = &git_args;
                    scope.spawn(move || {
                        for path in chunk_paths {
                            let output = std::process::Command::new("git")
                                .args(git_args)
                                .current_dir(path)
                                .output();
                            let result = match output {
                                Ok(out) => {
                                    let combined = format!(
                                        "{}{}",
                                        String::from_utf8_lossy(&out.stdout),
                                        String::from_utf8_lossy(&out.stderr)
                                    )
                                    .trim()
                                    .to_owned();
                                    if out.status.success() {
                                        ExecResult {
                                            path: path.to_string_lossy().into_owned(),
                                            success: true,
                                            stdout: combined,
                                            stderr: String::new(),
                                        }
                                    } else {
                                        ExecResult {
                                            path: path.to_string_lossy().into_owned(),
                                            success: false,
                                            stdout: String::new(),
                                            stderr: combined,
                                        }
                                    }
                                }
                                Err(err) => ExecResult {
                                    path: path.to_string_lossy().into_owned(),
                                    success: false,
                                    stdout: String::new(),
                                    stderr: err.to_string(),
                                },
                            };
                            if tx.send(Event::Msg(ExecMsg::Result(result))).is_err() {
                                break;
                            }
                        }
                    });
                }
            });
        });
}

fn convert_exec_key(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> super::sync::Key {
    use crossterm::event::{KeyCode, KeyModifiers};
    if modifiers.contains(KeyModifiers::CONTROL) {
        return match code {
            KeyCode::Char(c) => super::sync::Key::CtrlChar(c),
            _ => super::sync::Key::Esc,
        };
    }
    match code {
        KeyCode::Enter => super::sync::Key::Enter,
        KeyCode::Esc => super::sync::Key::Esc,
        KeyCode::Up => super::sync::Key::Up,
        KeyCode::Down => super::sync::Key::Down,
        KeyCode::PageUp => super::sync::Key::PgUp,
        KeyCode::PageDown => super::sync::Key::PgDown,
        KeyCode::Home => super::sync::Key::Home,
        KeyCode::End => super::sync::Key::End,
        KeyCode::Char(c) => super::sync::Key::Char(c),
        _ => super::sync::Key::Esc,
    }
}
