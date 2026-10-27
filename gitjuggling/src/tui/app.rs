//! The sync runtime (decision H): one blocking loop over one merged
//! channel. A key-reader thread converts crossterm events, a ticker drives
//! the spinner cadence, and worker threads (fetch/scan/execute) feed their
//! results back as messages — no async, no `EventStream`. Rendering goes
//! through an **inline** viewport (decision A) so the final summary frame
//! stays in terminal scrollback after exit, as in Go.
//!
//! `run` is backend-generic: production uses `CrosstermBackend`, the tests
//! drive it headless with `ratatui::backend::TestBackend` and a synthetic
//! event channel.

use std::path::Path;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::widgets::Paragraph;

use crate::discover;
use crate::exec;
use crate::git;

use super::sync::{Key, Msg, SyncApp};
use super::widgets::{SpinnerTick, TICK_INTERVAL};

/// Everything the event loop consumes, merged into one channel.
pub(crate) enum Event {
    Key(Key),
    Msg(Msg),
    Tick,
}

/// The production entry (Go: `cmd/sync.go`'s `tea.NewProgram(model).Run()`):
/// raw mode, an inline viewport, the reader/ticker threads, then restore.
pub fn run_sync(app: &mut SyncApp) -> anyhow::Result<()> {
    let (width, height) = terminal_size();
    let viewport_height = height.min(40);

    crossterm::terminal::enable_raw_mode()?;
    let mut terminal = ratatui::Terminal::with_options(
        CrosstermBackend::new(std::io::stdout()),
        ratatui::TerminalOptions {
            // Decision A: inline, not the alt screen, so the final summary
            // frame stays in terminal scrollback after exit, as in Go.
            viewport: ratatui::Viewport::Inline(viewport_height),
        },
    )?;
    terminal.hide_cursor()?;

    let result = (|| -> anyhow::Result<()> {
        let (tx, rx) = std::sync::mpsc::channel::<Event>();

        spawn_key_reader(tx.clone())?;
        spawn_ticker(tx.clone())?;
        tx.send(Event::Msg(Msg::Resized(width, viewport_height)))
            .ok();

        run(&mut terminal, app, rx)?;
        Ok(())
    })();

    terminal.show_cursor()?;
    crossterm::terminal::disable_raw_mode()?;
    result
}

fn terminal_size() -> (u16, u16) {
    crossterm::terminal::size().unwrap_or((80, 24))
}

fn spawn_key_reader(tx: Sender<Event>) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("tui-keys".into())
        .spawn(move || {
            loop {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(key)) => {
                        if key.kind == crossterm::event::KeyEventKind::Press
                            && tx.send(Event::Key(convert_key(key))).is_err()
                        {
                            break;
                        }
                    }
                    Ok(crossterm::event::Event::Resize(width, height)) => {
                        if tx
                            .send(Event::Msg(Msg::Resized(width, height.min(40))))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })?;
    Ok(())
}

fn spawn_ticker(tx: Sender<Event>) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("tui-tick".into())
        .spawn(move || {
            loop {
                thread::sleep(TICK_INTERVAL);
                if tx.send(Event::Tick).is_err() {
                    break;
                }
            }
        })?;
    Ok(())
}

fn convert_key(key: crossterm::event::KeyEvent) -> Key {
    use crossterm::event::{KeyCode, KeyModifiers};
    let code = key.code;
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match code {
            KeyCode::Char(c) => Key::CtrlChar(c),
            _ => Key::Esc,
        };
    }
    match code {
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::PageUp => Key::PgUp,
        KeyCode::PageDown => Key::PgDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Char(c) => Key::Char(c),
        _ => Key::Esc,
    }
}

/// The backend-agnostic event loop: feed the machine, run the commands it
/// asks for, redraw. Exits on `Cmd::Quit` or channel close.
pub(crate) fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut SyncApp,
    events: Receiver<Event>,
) -> anyhow::Result<()>
where
    B::Error: Send + Sync + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel::<Event>();
    // Bridge the caller's events into the loop's own channel so worker
    // threads spawned below share one Sender.
    let feeder = {
        let tx = tx.clone();
        thread::spawn(move || {
            for event in events {
                if tx.send(event).is_err() {
                    break;
                }
            }
        })
    };

    for cmd in app.init() {
        if execute_cmd(&tx, cmd)? {
            feeder.join().ok();
            return Ok(());
        }
    }

    let mut quit = false;
    while let Ok(event) = rx.recv() {
        let cmds = match event {
            Event::Key(key) => app.handle_key(key),
            Event::Msg(msg) => app.handle_msg(msg),
            Event::Tick => app.handle_msg(Msg::SpinnerTick(SpinnerTick)),
        };
        for cmd in cmds {
            if execute_cmd(&tx, cmd)? {
                quit = true;
                break;
            }
        }
        if quit {
            break;
        }
        draw(terminal, app)?;
    }
    feeder.join().ok();
    Ok(())
}

fn draw<B: Backend>(terminal: &mut Terminal<B>, app: &SyncApp) -> anyhow::Result<()>
where
    B::Error: Send + Sync + 'static,
{
    terminal.draw(|frame| {
        let area = frame.area();
        frame.render_widget(Paragraph::new(app.render()), area);
    })?;
    Ok(())
}

/// Runs one machine command on the runtime's behalf; returns true when the
/// loop must exit (`Cmd::Quit`).
fn execute_cmd(tx: &Sender<Event>, cmd: super::sync::Cmd) -> anyhow::Result<bool> {
    use super::sync::Cmd;
    match cmd {
        Cmd::Quit => return Ok(true),
        // The ticker thread drives the cadence continuously; the machine's
        // `Cmd::Tick` (bubbletea's re-arm pattern) is already covered.
        Cmd::Tick => {}
        Cmd::FetchGithub { owners, token } => {
            spawn_worker(tx.clone(), move || {
                let transport = crate::remote::UreqTransport::default();
                let result = crate::remote::fetch_github_repos(&transport, &owners, &token);
                Msg::GithubRepos(result)
            });
        }
        Cmd::FetchForgejo { url, user, token } => {
            spawn_worker(tx.clone(), move || {
                let transport = crate::remote::UreqTransport::default();
                let result = crate::remote::fetch_forgejo_repos(&transport, &url, &user, &token);
                Msg::ForgejoRepos(result)
            });
        }
        Cmd::ScanLocal { root } => {
            spawn_worker(tx.clone(), move || {
                let result = (|| -> anyhow::Result<_> {
                    let local = discover::discover(Path::new(&root))?;
                    let mut dirty = std::collections::HashMap::new();
                    for (_, repo) in local.iter() {
                        if let Ok(d) = git::is_dirty(&repo.path) {
                            dirty.insert(repo.path.to_string_lossy().into_owned(), d);
                        }
                    }
                    Ok((local, dirty))
                })();
                Msg::LocalRepos(result)
            });
        }
        Cmd::StartExecution {
            actions,
            dry_run,
            concurrency,
            github_token,
            cancel,
        } => {
            let tx = tx.clone();
            thread::Builder::new()
                .name("tui-exec".into())
                .spawn(move || {
                    let rx = exec::execute_actions(
                        &actions,
                        dry_run,
                        None,
                        concurrency,
                        &github_token,
                        &cancel,
                    );
                    for result in rx {
                        if tx.send(Event::Msg(Msg::ActionResult(result))).is_err() {
                            break;
                        }
                    }
                })?;
        }
    }
    Ok(false)
}

fn spawn_worker(tx: Sender<Event>, f: impl FnOnce() -> Msg + Send + 'static) {
    let _ = thread::Builder::new()
        .name("tui-worker".into())
        .spawn(move || {
            let msg = f();
            let _ = tx.send(Event::Msg(msg));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Action, ActionType};
    use crate::remote::{RemoteRepo, Source};
    use crate::state::Plan;
    use crate::tui::sync::{NullPlanStore, Phase};
    use ratatui::backend::TestBackend;
    use std::sync::Arc;

    /// Decision H's proof: the whole runtime runs headless. A dry-run
    /// resume from a saved plan walks Resume -> Enter -> Plan -> Enter ->
    /// execution (dry) -> summary -> quit, all through the merged-channel
    /// loop against a TestBackend; the executor's results arrive over the
    /// same channel from a real thread.
    #[test]
    fn run_drives_resume_to_summary_headless() {
        let repo = RemoteRepo {
            owner: "vincent".to_owned(),
            name: "demo".to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: String::new(),
            source: Source::GitHub,
        };
        let mut action = Action {
            action_type: ActionType::Clone,
            repo,
            local_path: Default::default(),
            current_path: Default::default(),
            expected_path: Default::default(),
            already_in_place: false,
            worktree_path: Default::default(),
            main_repo_path: Default::default(),
            branch: String::new(),
            merged: false,
        };
        action.expected_path = std::path::Path::new("/tmp/nowhere").into();
        let saved = Plan {
            actions: vec![action],
        };

        let mut app = SyncApp::new(
            "personal".to_owned(),
            crate::config::Workspace::default(),
            true, // dry run: the executor reports without touching /tmp/nowhere
            true,
            false,
            false,
            1,
            Some(saved),
        );
        app.with_seams(Arc::new(NullPlanStore::default()), None);

        let (tx, events) = std::sync::mpsc::channel::<Event>();
        let feeder = std::thread::spawn(move || {
            tx.send(Event::Msg(Msg::Resized(80, 24))).unwrap();
            tx.send(Event::Key(Key::Enter)).unwrap(); // resume
            tx.send(Event::Key(Key::Enter)).unwrap(); // execute (dry run)
        });

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        super::run(&mut terminal, &mut app, events).unwrap();
        feeder.join().unwrap();

        assert_eq!(app.phase, Phase::Summary);
        assert!(!app.has_failures(), "dry-run results are successes");
        assert_eq!(app.succeeded.len(), 1);
        assert_eq!(app.exec_log.len(), 1);
    }
}
