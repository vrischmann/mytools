//! The pure sync state machine (Go: `tui.SyncModel`) — decision H.
//!
//! `SyncApp` owns no terminal, no threads, and performs no IO except
//! through two injected seams: the [`PlanStore`] (durable-plan persistence,
//! Go calls `syncstate` inline) and the optional `branch_exists` probe Go
//! passes to `BuildPlan`. Everything else is data in, [`Cmd`]s out:
//! `handle_key`/`handle_msg` mirror bubbletea's `Model` contract so the
//! `tui/sync_test.go` ports run headless, exactly as they do in Go.
//!
//! Phased scope (RUST_REWRITE_PLAN.md §5): M5a implements loading, resume,
//! plan, the confirmation phase plumbing, execution start, and summary.
//! The tracking-fix and prune phases (their phase variants exist) and the
//! `main` wiring land in M5c.

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Workspace;
use crate::discover::LocalRepos;
use crate::exec::{ActionResult, CancelToken};
use crate::plan::{self, Action, ActionType, BranchExistsFn};
use crate::remote::{self, RemoteRepo};
use crate::state::Plan;
use crate::tui::PlanView;
use crate::tui::widgets::{Spinner, SpinnerTick};

/// One phase of the sync flow (Go: `syncPhase`). The tracking-fix and prune
/// phases are unreachable until M5c wires them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // the prune/tracking-fix variants are constructed by M5c
pub enum Phase {
    Loading,
    Resume,
    Plan,
    DirtyConfirm,
    MoveConfirm,
    WorktreeConfirm,
    Executing,
    Summary,
    PruneList,
    PruneConfirm,
    PruneDone,
    TrackingFix,
}

/// A key event, already normalized (Go: `tea.KeyMsg.String()` cases).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    /// e.g. ctrl+c
    CtrlChar(char),
    Up,
    Down,
    PgUp,
    PgDown,
    Home,
    End,
}

/// Messages the runtime feeds the machine (Go: the unexported `sync*Msg`
/// types plus `tea.WindowSizeMsg`, `spinner.TickMsg`).
pub enum Msg {
    GithubRepos(Result<Vec<RemoteRepo>, anyhow::Error>),
    ForgejoRepos(Result<Vec<RemoteRepo>, anyhow::Error>),
    LocalRepos(Result<(LocalRepos, HashMap<String, bool>), anyhow::Error>),
    ActionResult(ActionResult),
    SpinnerTick(SpinnerTick),
    Resized(u16, u16),
}

impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `LocalRepos` is not Debug; a coarse discriminant is enough for
        // diagnostics — nothing observable keys off this repr.
        let name = match self {
            Msg::GithubRepos(Ok(_)) => "GithubRepos(ok)",
            Msg::GithubRepos(Err(_)) => "GithubRepos(err)",
            Msg::ForgejoRepos(Ok(_)) => "ForgejoRepos(ok)",
            Msg::ForgejoRepos(Err(_)) => "ForgejoRepos(err)",
            Msg::LocalRepos(Ok(_)) => "LocalRepos(ok)",
            Msg::LocalRepos(Err(_)) => "LocalRepos(err)",
            Msg::ActionResult(_) => "ActionResult",
            Msg::SpinnerTick(_) => "SpinnerTick",
            Msg::Resized(..) => "Resized",
        };
        f.write_str(name)
    }
}

/// Commands the runtime executes on the machine's behalf (Go: `tea.Cmd`).
#[derive(Debug)]
pub enum Cmd {
    /// Exit the event loop (Go: `tea.Quit`).
    Quit,
    /// Resolve the GitHub listing (Go: `fetchGithubRepos`).
    FetchGithub { owners: Vec<String>, token: String },
    /// Resolve the Forgejo listing (Go: `fetchForgejoRepos`).
    FetchForgejo {
        url: String,
        user: String,
        token: String,
    },
    /// Walk the workspace root and detect dirty repos (Go: `scanLocalRepos`).
    ScanLocal { root: String },
    /// Keep the spinner ticking (Go: `spinner.Tick`).
    Tick,
    /// Spawn the bounded executor and stream results back as
    /// [`Msg::ActionResult`] (Go: `ExecuteActions` + `waitForActionResult`).
    StartExecution {
        actions: Vec<Action>,
        dry_run: bool,
        concurrency: usize,
        github_token: String,
        cancel: CancelToken,
    },
}

/// Durable-plan persistence seam (Go calls `syncstate.Save`/`Delete` inline
/// from `completeAction` and the resume/loading handlers).
pub trait PlanStore: Send + Sync {
    fn save(&self, workspace: &str, actions: &[Action]) -> anyhow::Result<()>;
    fn delete(&self, workspace: &str) -> anyhow::Result<()>;
}

/// The file-backed store used by the real runtime.
pub struct FilePlanStore;

impl PlanStore for FilePlanStore {
    fn save(&self, workspace: &str, actions: &[Action]) -> anyhow::Result<()> {
        crate::state::save(
            workspace,
            &Plan {
                actions: actions.to_vec(),
            },
        )
    }

    fn delete(&self, workspace: &str) -> anyhow::Result<()> {
        crate::state::delete(workspace)
    }
}

/// A no-op store for tests: records calls, never touches the filesystem.
#[cfg(test)]
#[derive(Default)]
pub struct NullPlanStore {
    pub saves: std::sync::Mutex<Vec<(String, usize)>>,
    pub deletes: std::sync::Mutex<Vec<String>>,
}

#[cfg(test)]
impl PlanStore for NullPlanStore {
    fn save(&self, workspace: &str, actions: &[Action]) -> anyhow::Result<()> {
        self.saves
            .lock()
            .unwrap()
            .push((workspace.to_owned(), actions.len()));
        Ok(())
    }

    fn delete(&self, workspace: &str) -> anyhow::Result<()> {
        self.deletes.lock().unwrap().push(workspace.to_owned());
        Ok(())
    }
}

pub(crate) struct LoadingStep {
    pub(crate) label: &'static str,
    pub(crate) done: bool,
}

/// The sync state machine (Go: `SyncModel`).
pub struct SyncApp {
    pub(crate) phase: Phase,
    pub(crate) workspace: String,

    // Options
    pub(crate) dry_run: bool,
    pub(crate) interactive: bool,
    // Read by M5c's prune routing (Go: the `--prune` option).
    #[allow(dead_code)]
    pub(crate) do_prune: bool,
    pub(crate) skip_pull: bool,
    pub(crate) concurrency: usize,

    // Config
    pub(crate) ws: Workspace,
    pub(crate) branch_exists: Option<Box<BranchExistsFn>>,

    // Loading phase
    pub(crate) spinner: Spinner,
    pub(crate) loading_steps: Vec<LoadingStep>,
    pub(crate) loading_idx: usize,

    // Data from loading
    github_repos: Vec<RemoteRepo>,
    forgejo_repos: Vec<RemoteRepo>,
    local_repos: Option<LocalRepos>,
    pub(crate) actions: Vec<Action>,
    pub(crate) saved_plan: Option<Plan>,

    // Plan phase
    pub(crate) plan_view: PlanView,
    pub(crate) updates: usize,
    pub(crate) skipped_pulls: usize,
    pub(crate) moves: usize,
    pub(crate) clones: usize,
    pub(crate) worktree_prunes: usize,

    // Confirmation phases
    move_confirm_indices: Vec<usize>,
    move_confirm_cursor: usize,
    skipped_move_indices: HashMap<usize, bool>,
    worktree_confirm_indices: Vec<usize>,
    worktree_confirm_cursor: usize,
    skipped_worktree_prunes: HashMap<usize, bool>,
    pub(crate) dirty_confirm_indices: Vec<usize>,
    dirty_confirm_cursor: usize,
    pub(crate) skipped_dirty_indices: HashMap<usize, bool>,
    local_dirty: HashMap<String, bool>,

    // Execution phase
    completed: usize,
    total: usize,
    exec_results: Vec<ActionResult>,
    pub(crate) exec_log: Vec<String>,

    // Summary phase
    pub(crate) succeeded: Vec<ActionResult>,
    pub(crate) failed: Vec<ActionResult>,

    // Error
    pub(crate) load_err: Option<anyhow::Error>,

    pub(crate) width: u16,
    pub(crate) height: u16,

    cancel: CancelToken,
    store: Arc<dyn PlanStore>,
}

impl SyncApp {
    /// Go: `NewSyncModel` — the eight parameters mirror the Go constructor
    /// one for one.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace: String,
        ws: Workspace,
        dry_run: bool,
        interactive: bool,
        do_prune: bool,
        skip_pull: bool,
        concurrency: usize,
        saved_plan: Option<Plan>,
    ) -> SyncApp {
        let phase = if saved_plan.is_some() {
            Phase::Resume
        } else {
            Phase::Loading
        };
        SyncApp {
            phase,
            workspace,
            dry_run,
            interactive,
            do_prune,
            skip_pull,
            concurrency,
            ws,
            branch_exists: None,
            spinner: Spinner::new(),
            loading_steps: vec![
                LoadingStep {
                    label: "Fetching GitHub repos...",
                    done: false,
                },
                LoadingStep {
                    label: "Fetching Forgejo repos...",
                    done: false,
                },
                LoadingStep {
                    label: "Scanning local repos...",
                    done: false,
                },
            ],
            loading_idx: 0,
            github_repos: Vec::new(),
            forgejo_repos: Vec::new(),
            local_repos: None,
            actions: Vec::new(),
            saved_plan,
            plan_view: PlanView::new(),
            updates: 0,
            skipped_pulls: 0,
            moves: 0,
            clones: 0,
            worktree_prunes: 0,
            move_confirm_indices: Vec::new(),
            move_confirm_cursor: 0,
            skipped_move_indices: HashMap::new(),
            worktree_confirm_indices: Vec::new(),
            worktree_confirm_cursor: 0,
            skipped_worktree_prunes: HashMap::new(),
            dirty_confirm_indices: Vec::new(),
            dirty_confirm_cursor: 0,
            skipped_dirty_indices: HashMap::new(),
            local_dirty: HashMap::new(),
            completed: 0,
            total: 0,
            exec_results: Vec::new(),
            exec_log: Vec::new(),
            succeeded: Vec::new(),
            failed: Vec::new(),
            load_err: None,
            width: 0,
            height: 0,
            cancel: CancelToken::new(),
            store: Arc::new(FilePlanStore),
        }
    }

    /// Replace the persistence seam and the upstream-branch probe (tests
    /// inject a [`NullPlanStore`] and no probe; the runtime injects git).
    pub fn with_seams(
        &mut self,
        store: Arc<dyn PlanStore>,
        branch_exists: Option<Box<BranchExistsFn>>,
    ) {
        self.store = store;
        self.branch_exists = branch_exists;
    }

    /// Go: `Init`.
    pub fn init(&mut self) -> Vec<Cmd> {
        if self.phase == Phase::Resume {
            return Vec::new();
        }
        vec![Cmd::Tick, self.fetch_github()]
    }

    fn fetch_github(&self) -> Cmd {
        Cmd::FetchGithub {
            owners: self.ws.github_owners.clone(),
            token: self.ws.github_token.clone(),
        }
    }

    fn fetch_forgejo(&self) -> Cmd {
        Cmd::FetchForgejo {
            url: self.ws.forgejo_url.clone(),
            user: self.ws.forgejo_user.clone(),
            token: self.ws.forgejo_token.clone(),
        }
    }

    fn scan_local(&self) -> Cmd {
        Cmd::ScanLocal {
            root: self.ws.root.clone(),
        }
    }

    /// Go: `Update`. Dispatches a message, mutating state and returning the
    /// commands the runtime must run.
    pub fn handle_msg(&mut self, msg: Msg) -> Vec<Cmd> {
        match msg {
            Msg::Resized(width, height) => {
                self.width = width;
                self.height = height;
                self.plan_view
                    .resize(width as usize, height.saturating_sub(2) as usize);
                Vec::new()
            }
            Msg::SpinnerTick(_) => {
                if self.phase == Phase::Loading {
                    self.spinner.advance();
                    vec![Cmd::Tick]
                } else {
                    Vec::new()
                }
            }
            Msg::GithubRepos(Ok(repos)) => {
                self.github_repos = repos;
                self.loading_steps[0].done = true;
                self.loading_idx = 1;
                vec![self.fetch_forgejo()]
            }
            Msg::GithubRepos(Err(err)) => self.load_failed(err),
            Msg::ForgejoRepos(Ok(repos)) => {
                self.forgejo_repos = repos;
                self.loading_steps[1].done = true;
                self.loading_idx = 2;
                vec![self.scan_local()]
            }
            Msg::ForgejoRepos(Err(err)) => self.load_failed(err),
            Msg::LocalRepos(Ok((local, dirty))) => {
                self.loading_steps[2].done = true;
                self.local_dirty = dirty;

                // Go dedups GitHub over Forgejo before building the plan.
                let (github, forgejo) =
                    remote::dedup_repos(self.github_repos.clone(), self.forgejo_repos.clone());
                self.github_repos = github;
                self.forgejo_repos = forgejo;

                let all: Vec<RemoteRepo> = self
                    .github_repos
                    .iter()
                    .chain(self.forgejo_repos.iter())
                    .cloned()
                    .collect();
                self.local_repos = Some(local);
                let local_ref = self.local_repos.as_ref().expect("just set");
                let actions = match plan::build_plan(
                    &all,
                    local_ref,
                    &self.ws,
                    self.branch_exists.as_deref(),
                ) {
                    Ok(actions) => actions,
                    Err(err) => return self.load_failed(err),
                };
                self.set_plan(actions);

                if !self.dry_run {
                    if self.actions.is_empty() {
                        if let Err(err) = self.store.delete(&self.workspace) {
                            return self.load_failed(err);
                        }
                    } else {
                        let plan = Plan {
                            actions: self.actions.clone(),
                        };
                        if let Err(err) = self.store.save(&self.workspace, &plan.actions) {
                            return self.load_failed(err);
                        }
                    }
                }

                self.phase = Phase::Plan;
                self.plan_view.set_content(self.build_plan_lines());
                Vec::new()
            }
            Msg::LocalRepos(Err(err)) => self.load_failed(err),
            Msg::ActionResult(result) => {
                self.completed += 1;
                self.exec_results.push(result.clone());
                self.exec_log.push(format_action_result(&result));
                if let Err(err) = self.complete_action(&result) {
                    self.load_err = Some(err);
                    self.phase = Phase::Summary;
                    return vec![Cmd::Quit];
                }
                if self.completed >= self.total {
                    return self.finish_execution();
                }
                Vec::new()
            }
        }
    }

    fn load_failed(&mut self, err: anyhow::Error) -> Vec<Cmd> {
        self.load_err = Some(err);
        self.phase = Phase::Summary;
        vec![Cmd::Quit]
    }

    /// Go: `handleKey`.
    pub fn handle_key(&mut self, key: Key) -> Vec<Cmd> {
        match self.phase {
            Phase::Resume => self.handle_resume_key(key),
            Phase::Plan => self.handle_plan_key(key),
            Phase::DirtyConfirm => self.handle_confirm_key(key, ConfirmKind::Dirty),
            Phase::MoveConfirm => self.handle_confirm_key(key, ConfirmKind::Move),
            Phase::WorktreeConfirm => self.handle_confirm_key(key, ConfirmKind::Worktree),
            Phase::Executing => {
                if matches!(key, Key::Char('q') | Key::CtrlChar('c')) {
                    self.cancel.cancel();
                    vec![Cmd::Quit]
                } else {
                    Vec::new()
                }
            }
            Phase::Summary => {
                if matches!(key, Key::Char('q') | Key::Enter | Key::CtrlChar('c')) {
                    vec![Cmd::Quit]
                } else {
                    Vec::new()
                }
            }
            // Prune and tracking-fix phases are wired in M5c; until then
            // they are unreachable.
            Phase::PruneList | Phase::PruneConfirm | Phase::PruneDone | Phase::TrackingFix => {
                Vec::new()
            }
            Phase::Loading => Vec::new(),
        }
    }

    fn handle_resume_key(&mut self, key: Key) -> Vec<Cmd> {
        match key {
            Key::Char('y' | 'Y') | Key::Enter => {
                let plan = self
                    .saved_plan
                    .clone()
                    .expect("resume phase implies a saved plan");
                self.set_plan(plan.actions);
                self.phase = Phase::Plan;
                self.plan_view.set_content(self.build_plan_lines());
                Vec::new()
            }
            Key::Char('n' | 'N') => {
                if let Err(err) = self.store.delete(&self.workspace) {
                    self.load_err = Some(err);
                    self.phase = Phase::Summary;
                    return vec![Cmd::Quit];
                }
                self.saved_plan = None;
                self.phase = Phase::Loading;
                vec![Cmd::Tick, self.fetch_github()]
            }
            Key::Char('q') | Key::CtrlChar('c') | Key::Esc => {
                self.cancel.cancel();
                vec![Cmd::Quit]
            }
            _ => Vec::new(),
        }
    }

    fn handle_plan_key(&mut self, key: Key) -> Vec<Cmd> {
        match key {
            Key::Enter => self.enter_from_plan(),
            Key::Char('q') | Key::CtrlChar('c') => {
                self.cancel.cancel();
                vec![Cmd::Quit]
            }
            _ => {
                self.plan_view.handle_key(key);
                Vec::new()
            }
        }
    }

    fn handle_confirm_key(&mut self, key: Key, kind: ConfirmKind) -> Vec<Cmd> {
        let (len, cursor) = match kind {
            ConfirmKind::Dirty => (self.dirty_confirm_indices.len(), self.dirty_confirm_cursor),
            ConfirmKind::Move => (self.move_confirm_indices.len(), self.move_confirm_cursor),
            ConfirmKind::Worktree => (
                self.worktree_confirm_indices.len(),
                self.worktree_confirm_cursor,
            ),
        };
        let current_index = match kind {
            ConfirmKind::Dirty => self.dirty_confirm_indices.get(cursor).copied(),
            ConfirmKind::Move => self.move_confirm_indices.get(cursor).copied(),
            ConfirmKind::Worktree => self.worktree_confirm_indices.get(cursor).copied(),
        };
        let new_cursor = match key {
            Key::Char('y' | 'Y') | Key::Enter => cursor + 1,
            Key::Char('n' | 'N') => {
                if let Some(i) = current_index {
                    match kind {
                        ConfirmKind::Dirty => {
                            self.skipped_dirty_indices.insert(i, true);
                        }
                        ConfirmKind::Move => {
                            self.skipped_move_indices.insert(i, true);
                        }
                        ConfirmKind::Worktree => {
                            self.skipped_worktree_prunes.insert(i, true);
                        }
                    }
                }
                cursor + 1
            }
            Key::Char('a' | 'A') => len,
            Key::Char('q') | Key::Esc | Key::CtrlChar('c') => {
                self.phase = Phase::Plan;
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        self.advance_confirm(kind, new_cursor);
        if new_cursor >= len {
            return match kind {
                ConfirmKind::Dirty => self.enter_move_or_execute(),
                ConfirmKind::Move => self.enter_worktree_or_execute(),
                ConfirmKind::Worktree => self.start_execution(
                    self.build_executable_actions(),
                    self.build_initial_results(),
                ),
            };
        }
        Vec::new()
    }

    fn advance_confirm(&mut self, kind: ConfirmKind, cursor: usize) {
        match kind {
            ConfirmKind::Dirty => self.dirty_confirm_cursor = cursor,
            ConfirmKind::Move => self.move_confirm_cursor = cursor,
            ConfirmKind::Worktree => self.worktree_confirm_cursor = cursor,
        }
    }

    /// Go: `enterFromPlan` — interactive non-dry runs walk the confirmation
    /// phases (dirty first, then moves, then worktree prunes) before
    /// executing; everything else starts execution immediately.
    fn enter_from_plan(&mut self) -> Vec<Cmd> {
        if self.dry_run || !self.interactive {
            return self.start_execution(
                self.build_executable_actions(),
                self.build_initial_results(),
            );
        }
        let dirty = self.dirty_update_indices();
        if !dirty.is_empty() {
            self.phase = Phase::DirtyConfirm;
            self.dirty_confirm_indices = dirty;
            self.dirty_confirm_cursor = 0;
            self.skipped_dirty_indices = HashMap::new();
            return Vec::new();
        }
        self.enter_move_or_execute()
    }

    fn enter_move_or_execute(&mut self) -> Vec<Cmd> {
        if self.moves > 0 {
            self.phase = Phase::MoveConfirm;
            self.move_confirm_indices = self
                .actions
                .iter()
                .enumerate()
                .filter(|(_, a)| a.action_type == ActionType::Move)
                .map(|(i, _)| i)
                .collect();
            self.move_confirm_cursor = 0;
            self.skipped_move_indices = HashMap::new();
            return Vec::new();
        }
        self.enter_worktree_or_execute()
    }

    fn enter_worktree_or_execute(&mut self) -> Vec<Cmd> {
        if self.worktree_prunes > 0 {
            self.phase = Phase::WorktreeConfirm;
            self.worktree_confirm_indices = self
                .actions
                .iter()
                .enumerate()
                .filter(|(_, a)| a.action_type == ActionType::PruneWorktree)
                .map(|(i, _)| i)
                .collect();
            self.worktree_confirm_cursor = 0;
            self.skipped_worktree_prunes = HashMap::new();
            return Vec::new();
        }
        self.start_execution(
            self.build_executable_actions(),
            self.build_initial_results(),
        )
    }

    /// Go: `startExecution`.
    fn start_execution(
        &mut self,
        actions: Vec<Action>,
        initial_results: Vec<ActionResult>,
    ) -> Vec<Cmd> {
        self.phase = Phase::Executing;
        self.total = actions.len() + initial_results.len();
        self.completed = initial_results.len();
        self.exec_results = initial_results.clone();
        self.exec_log = initial_results.iter().map(format_action_result).collect();
        self.succeeded.clear();
        self.failed.clear();
        for result in &initial_results {
            if let Err(err) = self.complete_action(result) {
                self.load_err = Some(err);
                self.phase = Phase::Summary;
                return vec![Cmd::Quit];
            }
        }

        if actions.is_empty() {
            return self.finish_execution();
        }

        vec![Cmd::StartExecution {
            actions,
            dry_run: self.dry_run,
            concurrency: self.concurrency,
            github_token: self.ws.github_token.clone(),
            cancel: self.cancel.clone(),
        }]
    }

    /// Go: `finishExecution`, minus the tracking-fix and prune routing that
    /// M5c adds: split results, then summary + quit.
    fn finish_execution(&mut self) -> Vec<Cmd> {
        for r in &self.exec_results {
            if r.success {
                self.succeeded.push(r.clone());
            } else {
                self.failed.push(r.clone());
            }
        }
        self.phase = Phase::Summary;
        vec![Cmd::Quit]
    }

    /// Go: `setPlan` — store the actions and count each kind.
    fn set_plan(&mut self, actions: Vec<Action>) {
        self.actions = actions;
        self.updates = 0;
        self.skipped_pulls = 0;
        self.moves = 0;
        self.clones = 0;
        self.worktree_prunes = 0;
        for a in &self.actions {
            match a.action_type {
                ActionType::Update => {
                    if self.skip_pull && a.already_in_place {
                        self.skipped_pulls += 1;
                    } else {
                        self.updates += 1;
                    }
                }
                ActionType::Move => self.moves += 1,
                ActionType::Clone => self.clones += 1,
                ActionType::PruneWorktree => self.worktree_prunes += 1,
            }
        }
    }

    /// Go: `completeAction` — remove a successful action from the durable
    /// plan; an empty plan deletes the file.
    fn complete_action(&mut self, result: &ActionResult) -> anyhow::Result<()> {
        if !result.success || self.dry_run {
            return Ok(());
        }
        let path_of = |action: &Action| -> String {
            let path = match action.action_type {
                ActionType::Update => action.local_path.clone(),
                ActionType::PruneWorktree => action.worktree_path.clone(),
                ActionType::Move => {
                    if result.message == "skipped (user declined)" {
                        action.current_path.clone()
                    } else {
                        action.expected_path.clone()
                    }
                }
                _ => action.expected_path.clone(),
            };
            path.to_string_lossy().into_owned()
        };
        let Some(pos) = self.actions.iter().position(|a| path_of(a) == result.path) else {
            return Ok(());
        };
        self.actions.remove(pos);
        if self.actions.is_empty() {
            self.store.delete(&self.workspace)?;
        } else {
            let plan = Plan {
                actions: self.actions.clone(),
            };
            self.store.save(&self.workspace, &plan.actions)?;
        }
        Ok(())
    }

    /// Go: `buildExecutableActions`.
    pub fn build_executable_actions(&self) -> Vec<Action> {
        self.actions
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                !(a.action_type == ActionType::Move
                    && self.skipped_move_indices.get(i).copied().unwrap_or(false)
                    || a.action_type == ActionType::PruneWorktree
                        && self
                            .skipped_worktree_prunes
                            .get(i)
                            .copied()
                            .unwrap_or(false)
                    || a.action_type == ActionType::Update && self.skip_pull && a.already_in_place
                    || a.action_type == ActionType::Update
                        && self.skipped_dirty_indices.get(i).copied().unwrap_or(false))
            })
            .map(|(_, a)| a.clone())
            .collect()
    }

    /// Go: `buildSkippedPullResults`.
    pub fn build_skipped_pull_results(&self) -> Vec<ActionResult> {
        if !self.skip_pull {
            return Vec::new();
        }
        self.actions
            .iter()
            .filter(|a| a.action_type == ActionType::Update && a.already_in_place)
            .map(|a| ActionResult {
                description: repo_desc(a),
                path: a.local_path.to_string_lossy().into_owned(),
                success: true,
                message: "skipped (pull disabled)".to_owned(),
            })
            .collect()
    }

    /// Go: `buildSkippedMoveResults`.
    pub fn build_skipped_move_results(&self) -> Vec<ActionResult> {
        if self.skipped_move_indices.is_empty() {
            return Vec::new();
        }
        self.actions
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                a.action_type == ActionType::Move
                    && self.skipped_move_indices.get(i).copied().unwrap_or(false)
            })
            .map(|(_, a)| ActionResult {
                description: repo_desc(a),
                path: a.current_path.to_string_lossy().into_owned(),
                success: true,
                message: "skipped (user declined)".to_owned(),
            })
            .collect()
    }

    /// Go: `buildSkippedWorktreeResults`.
    pub fn build_skipped_worktree_results(&self) -> Vec<ActionResult> {
        if self.skipped_worktree_prunes.is_empty() {
            return Vec::new();
        }
        self.actions
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                a.action_type == ActionType::PruneWorktree
                    && self
                        .skipped_worktree_prunes
                        .get(i)
                        .copied()
                        .unwrap_or(false)
            })
            .map(|(_, a)| ActionResult {
                description: repo_desc(a),
                path: a.worktree_path.to_string_lossy().into_owned(),
                success: true,
                message: "skipped (user declined)".to_owned(),
            })
            .collect()
    }

    /// Go: `buildSkippedDirtyResults`.
    pub fn build_skipped_dirty_results(&self) -> Vec<ActionResult> {
        if self.skipped_dirty_indices.is_empty() {
            return Vec::new();
        }
        self.actions
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                a.action_type == ActionType::Update
                    && self.skipped_dirty_indices.get(i).copied().unwrap_or(false)
            })
            .map(|(_, a)| ActionResult {
                description: repo_desc(a),
                path: a.local_path.to_string_lossy().into_owned(),
                success: true,
                message: "skipped (uncommitted changes — user declined)".to_owned(),
            })
            .collect()
    }

    /// Go: `buildInitialResults`.
    pub fn build_initial_results(&self) -> Vec<ActionResult> {
        let mut results = self.build_skipped_pull_results();
        results.extend(self.build_skipped_dirty_results());
        results.extend(self.build_skipped_move_results());
        results.extend(self.build_skipped_worktree_results());
        results
    }

    /// Go: `dirtyUpdateIndices` — indices of update actions that will run
    /// and whose local tree has uncommitted changes.
    pub fn dirty_update_indices(&self) -> Vec<usize> {
        self.actions
            .iter()
            .enumerate()
            .filter(|(_i, a)| {
                a.action_type == ActionType::Update
                    && !(self.skip_pull && a.already_in_place)
                    && self
                        .local_dirty
                        .get(&a.local_path.to_string_lossy().into_owned())
                        .copied()
                        .unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Go: `HasFailures` — drives the process exit code (§4.11).
    pub fn has_failures(&self) -> bool {
        !self.failed.is_empty()
    }

    /// The workspace name shown in headings ("" renders as "default").
    pub fn workspace_label(&self) -> &str {
        if self.workspace.is_empty() {
            "default"
        } else {
            &self.workspace
        }
    }

    /// The machine's cancel token; the runtime passes it to the executor.
    pub fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
}

#[derive(Clone, Copy)]
enum ConfirmKind {
    Dirty,
    Move,
    Worktree,
}

fn repo_desc(action: &Action) -> String {
    format!(
        "{}/{} ({})",
        action.repo.owner,
        action.repo.name,
        action.repo.source_label()
    )
}

/// Go: `formatActionResult` (sync.go:1392) — the executing-phase log line.
pub fn format_action_result(r: &ActionResult) -> String {
    format!("{} {}", r.description, r.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_repo(owner: &str, name: &str, source: crate::remote::Source) -> RemoteRepo {
        RemoteRepo {
            owner: owner.to_owned(),
            name: name.to_owned(),
            is_fork: false,
            is_archived: false,
            is_mirror: false,
            clone_url: String::new(),
            source,
        }
    }

    fn action(action_type: ActionType, repo: &RemoteRepo) -> Action {
        let mut a = Action {
            action_type,
            repo: repo.clone(),
            local_path: Default::default(),
            current_path: Default::default(),
            expected_path: Default::default(),
            already_in_place: false,
            worktree_path: Default::default(),
            main_repo_path: Default::default(),
            branch: String::new(),
            merged: false,
        };
        match action_type {
            ActionType::Update => a.local_path = Path::new("/tmp/x").into(),
            ActionType::Move => {
                a.current_path = Path::new("/tmp/x").into();
                a.expected_path = Path::new("/tmp/y").into();
            }
            ActionType::Clone => a.expected_path = Path::new("/tmp/x").into(),
            ActionType::PruneWorktree => a.worktree_path = Path::new("/tmp/x").into(),
        }
        a
    }

    fn null_store() -> Arc<NullPlanStore> {
        Arc::new(NullPlanStore::default())
    }

    fn test_app() -> SyncApp {
        let mut app = SyncApp::new(
            "personal".to_owned(),
            crate::config::Workspace::default(),
            false,
            true,
            false,
            false,
            1,
            None,
        );
        app.with_seams(null_store(), None);
        app
    }

    // go test-run: internal/tui/sync_test.go TestBuildExecutableActionsSkipsDeclinedMoves
    #[test]
    fn test_build_executable_actions_skips_declined_moves() {
        let repo = test_repo("vincent", "demo", crate::remote::Source::GitHub);
        let mut app = test_app();
        app.actions = vec![
            action(ActionType::Update, &repo),
            action(ActionType::Move, &repo),
            action(ActionType::Clone, &repo),
        ];
        app.skipped_move_indices.insert(1, true);

        let actions = app.build_executable_actions();
        assert_eq!(actions.len(), 2, "declined move drops out");
        assert_eq!(actions[0].action_type, ActionType::Update);
        assert_eq!(actions[1].action_type, ActionType::Clone);

        let results = app.build_skipped_move_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "/tmp/x");
        assert_eq!(results[0].message, "skipped (user declined)");
    }

    // go test-run: internal/tui/sync_test.go TestResumePlanUsesSavedActions
    #[test]
    fn test_resume_plan_uses_saved_actions() {
        let repo = test_repo("vincent", "demo", crate::remote::Source::GitHub);
        let mut a = action(ActionType::Clone, &repo);
        a.expected_path = Path::new("/tmp/demo").into();
        let saved = Plan { actions: vec![a] };
        let mut app = SyncApp::new(
            "personal".to_owned(),
            crate::config::Workspace::default(),
            false,
            true,
            false,
            false,
            1,
            Some(saved),
        );
        app.with_seams(null_store(), None);

        app.handle_key(Key::Enter);
        assert_eq!(app.phase, Phase::Plan);
        assert_eq!(app.actions.len(), 1);
        assert_eq!(app.clones, 1);
    }

    // go test-run: internal/tui/sync_test.go TestBuildExecutableActionsSkipsPullForReposAlreadyInPlace
    #[test]
    fn test_build_executable_actions_skips_pull_for_repos_already_in_place() {
        let repo = test_repo("vincent", "demo", crate::remote::Source::GitHub);
        let mut app = test_app();
        app.skip_pull = true;
        let in_place = {
            let mut a = action(ActionType::Update, &repo);
            a.local_path = Path::new("/tmp/in-place").into();
            a.expected_path = Path::new("/tmp/in-place").into();
            a.already_in_place = true;
            a
        };
        let mut misplaced = action(ActionType::Update, &repo);
        misplaced.local_path = Path::new("/tmp/misplaced").into();
        misplaced.expected_path = Path::new("/tmp/expected").into();
        app.actions = vec![in_place, misplaced, action(ActionType::Clone, &repo)];
        app.skipped_pulls = 1;

        let actions = app.build_executable_actions();
        assert_eq!(actions.len(), 2);
        assert_eq!(
            actions[0].local_path,
            Path::new("/tmp/misplaced"),
            "misplaced repo update stays executable"
        );
        assert_eq!(actions[1].action_type, ActionType::Clone);

        let results = app.build_skipped_pull_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "/tmp/in-place");
        assert_eq!(results[0].message, "skipped (pull disabled)");
    }

    // go test-run: internal/tui/sync_test.go TestStartExecutionWithOnlySkippedMovesFinishesImmediately
    #[test]
    fn test_start_execution_with_only_skipped_moves_finishes_immediately() {
        let mut app = test_app();
        app.start_execution(
            Vec::new(),
            vec![ActionResult {
                description: String::new(),
                path: String::new(),
                success: true,
                message: "skipped (user declined)".to_owned(),
            }],
        );
        assert_eq!(app.phase, Phase::Summary);
    }

    // go test-run: internal/tui/sync_test.go TestBuildExecutableActionsSkipsDeclinedDirtyUpdates
    #[test]
    fn test_build_executable_actions_skips_declined_dirty_updates() {
        let repo = test_repo("vincent", "demo", crate::remote::Source::GitHub);
        let mut app = test_app();
        let mut clean = action(ActionType::Update, &repo);
        clean.local_path = Path::new("/tmp/clean").into();
        let mut dirty = action(ActionType::Update, &repo);
        dirty.local_path = Path::new("/tmp/dirty").into();
        app.actions = vec![clean, dirty, action(ActionType::Clone, &repo)];
        app.skipped_dirty_indices.insert(1, true);

        let actions = app.build_executable_actions();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].local_path, Path::new("/tmp/clean"));
        assert_eq!(actions[1].action_type, ActionType::Clone);

        let results = app.build_skipped_dirty_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "/tmp/dirty");
        assert!(results[0].message.contains("uncommitted changes"));
    }

    // go test-run: internal/tui/sync_test.go TestDirtyUpdateIndicesFlagsOnlyExecutableDirtyUpdates
    #[test]
    fn test_dirty_update_indices_flags_only_executable_dirty_updates() {
        let repo = test_repo("vincent", "demo", crate::remote::Source::GitHub);
        let mut app = test_app();
        app.skip_pull = true;
        app.local_dirty = [
            ("/tmp/clean".to_owned(), false),
            ("/tmp/dirty".to_owned(), true),
            ("/tmp/skip".to_owned(), true),
        ]
        .into_iter()
        .collect();
        let mk = |p: &str, in_place: bool| {
            let mut a = action(ActionType::Update, &repo);
            a.local_path = Path::new(p).into();
            a.already_in_place = in_place;
            a
        };
        app.actions = vec![
            mk("/tmp/clean", false),
            mk("/tmp/dirty", false),
            // Already in place with --skip-pull: would not execute, excluded.
            mk("/tmp/skip", true),
            action(ActionType::Clone, &repo),
        ];

        assert_eq!(app.dirty_update_indices(), vec![1]);
    }

    // go test-run: internal/tui/sync_test.go TestDirtyConfirmKeysSkipAndProceed
    #[test]
    fn test_dirty_confirm_keys_skip_and_proceed() {
        let repo = test_repo("vincent", "demo", crate::remote::Source::GitHub);
        let mut app = test_app();
        app.interactive = true;
        app.local_dirty = [
            ("/tmp/dirty1".to_owned(), true),
            ("/tmp/dirty2".to_owned(), true),
        ]
        .into_iter()
        .collect();
        let mk = |p: &str| {
            let mut a = action(ActionType::Update, &repo);
            a.local_path = Path::new(p).into();
            a
        };
        app.actions = vec![mk("/tmp/dirty1"), mk("/tmp/dirty2")];
        app.phase = Phase::Plan;

        // Enter plan -> enters dirty confirm over both dirty updates.
        app.handle_key(Key::Enter);
        assert_eq!(app.phase, Phase::DirtyConfirm);
        assert_eq!(app.dirty_confirm_indices, vec![0, 1]);

        // Skip the first: it drops out of the executable set.
        app.handle_key(Key::Char('n'));
        assert!(app.skipped_dirty_indices.get(&0).copied().unwrap_or(false));
        let update_paths: Vec<String> = app
            .build_executable_actions()
            .iter()
            .filter(|a| a.action_type == ActionType::Update)
            .map(|a| a.local_path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(update_paths, vec!["/tmp/dirty2"]);

        // Confirm the second: no more dirty items, so it proceeds to
        // execution.
        app.handle_key(Key::Char('y'));
        assert_eq!(app.phase, Phase::Executing);
    }

    // go test-run: internal/tui/sync_test.go TestBuildExecutableActionsSkipsDeclinedWorktreePrunes
    #[test]
    fn test_build_executable_actions_skips_declined_worktree_prunes() {
        let repo = test_repo("vincent", "ppst", crate::remote::Source::Forgejo);
        let mut app = test_app();
        let mut prune = action(ActionType::PruneWorktree, &repo);
        prune.worktree_path = Path::new("/tmp/ppst-iter-seq").into();
        prune.main_repo_path = Path::new("/tmp/ppst").into();
        prune.branch = "iter-seq".to_owned();
        app.actions = vec![action(ActionType::Update, &repo), prune];
        app.skipped_worktree_prunes.insert(1, true);

        let actions = app.build_executable_actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action_type, ActionType::Update);

        let results = app.build_skipped_worktree_results();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "/tmp/ppst-iter-seq");
        assert_eq!(results[0].message, "skipped (user declined)");
    }

    // Extra beyond the Go set: a cancelled token is visible to the runner
    // (q stops scheduling; in-flight subprocesses run to completion).
    #[test]
    fn test_quit_key_cancels_token() {
        let mut app = test_app();
        app.phase = Phase::Executing;
        let cmds = app.handle_key(Key::Char('q'));
        assert!(matches!(cmds.as_slice(), [Cmd::Quit]));
        assert!(app.cancel_token().is_cancelled());
    }
}
