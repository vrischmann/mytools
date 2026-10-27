//! Phase rendering (Go: the `renderX` half of `tui/sync.go`) — decision A:
//! idiomatic ratatui lines, not Bubbletea render-to-string. M5a renders
//! loading, resume, plan, and summary; the confirmation, executing,
//! tracking-fix, and prune views land with M5b/M5c.

use ratatui::text::{Line, Span};

use super::SyncApp;
use super::styles::{self, dim, error, label, success, warn};
use super::sync::Phase;
use crate::plan::ActionType;

impl SyncApp {
    /// The current frame as styled lines (Go: `View()`).
    pub fn render(&self) -> Vec<Line<'static>> {
        match self.phase {
            Phase::Loading => self.render_loading(),
            Phase::Resume => self.render_resume(),
            Phase::Plan => self.render_plan(),
            Phase::DirtyConfirm => self.render_dirty_confirm(),
            Phase::MoveConfirm => self.render_move_confirm(),
            Phase::WorktreeConfirm => self.render_worktree_confirm(),
            Phase::Executing => self.render_executing(),
            Phase::Summary => self.render_summary(),
            Phase::TrackingFix => self.render_tracking_fix(),
            Phase::PruneList => self.render_prune_list(),
            Phase::PruneConfirm => self.render_prune_confirm(),
            Phase::PruneDone => self.render_prune_done(),
        }
    }

    /// Go: `renderResume`.
    fn render_resume(&self) -> Vec<Line<'static>> {
        let count = self
            .saved_plan
            .as_ref()
            .map(|p| p.actions.len())
            .unwrap_or(0);
        vec![
            styles::section_header("Resume saved plan?"),
            Line::from(""),
            Line::from(format!(
                "  An unfinished sync plan with {count} pending actions was found."
            )),
            Line::from(""),
            Line::from(Span::styled(
                "[Enter/y] Resume  [n] Rebuild  [q] Quit".to_owned(),
                dim(),
            )),
        ]
    }

    /// Go: `renderLoading`.
    fn render_loading(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            styles::section_header(&format!("Syncing workspace: {}", self.workspace_label())),
            Line::from(""),
        ];
        for (i, step) in self.loading_steps.iter().enumerate() {
            if step.done {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    styles::checkmark(),
                    Span::raw(format!(" {}", step.label)),
                ]));
            } else if i == self.loading_idx {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    styles::bullet(),
                    Span::raw(format!(" {} ", step.label)),
                    Span::styled(self.spinner.frame().to_owned(), success()),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    styles::pending_bullet(),
                    Span::raw(format!(" {}", step.label)),
                ]));
            }
        }
        lines
    }

    /// Go: `renderPlan` — the scrollable plan viewport.
    fn render_plan(&self) -> Vec<Line<'static>> {
        self.plan_view.visible().to_vec()
    }

    /// Go: `buildPlanContent` — the plan view's content, as styled lines.
    pub(crate) fn build_plan_lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            styles::section_header("Plan"),
            Line::from(""),
            Line::from(""),
        ];

        // "  <counts>[, pulls skipped][, stale worktrees]"
        let mut head = vec![Span::raw("  ")];
        head.extend(styles::count_display(self.updates, self.moves, self.clones));
        let any_counts = self.updates > 0 || self.moves > 0 || self.clones > 0;
        if self.skipped_pulls > 0 {
            if any_counts {
                head.push(Span::raw(", "));
            }
            head.push(Span::styled(self.skipped_pulls.to_string(), success()));
            head.push(Span::styled(" pulls skipped".to_owned(), label()));
        }
        if self.worktree_prunes > 0 {
            if any_counts || self.skipped_pulls > 0 {
                head.push(Span::raw(", "));
            }
            head.push(Span::styled(self.worktree_prunes.to_string(), warn()));
            head.push(Span::styled(" stale worktrees".to_owned(), label()));
        }
        lines.push(Line::from(head));

        if self.updates > 0 {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("    {} repos will get a git pull --rebase", self.updates),
                dim(),
            )));
        }
        if self.skipped_pulls > 0 {
            lines.push(Line::from(vec![
                Span::raw("    "),
                styles::checkmark(),
                Span::styled(
                    format!(
                        " {} repos are already in the expected location (pull skipped)",
                        self.skipped_pulls
                    ),
                    dim(),
                ),
            ]));
        }
        if self.moves > 0 {
            lines.push(Line::from(""));
            for a in &self.actions {
                if a.action_type == ActionType::Move {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        styles::arrow(),
                        Span::raw(format!(
                            " {}/{} ({})  ",
                            a.repo.owner,
                            a.repo.name,
                            a.repo.source_label()
                        )),
                        Span::styled(a.current_path.to_string_lossy().into_owned(), dim()),
                        Span::raw(" → "),
                        Span::styled(a.expected_path.to_string_lossy().into_owned(), dim()),
                    ]));
                }
            }
        }
        if self.worktree_prunes > 0 {
            lines.push(Line::from(""));
            for a in &self.actions {
                if a.action_type == ActionType::PruneWorktree {
                    let worktree =
                        Span::styled(a.worktree_path.to_string_lossy().into_owned(), dim());
                    let branch = Span::styled(a.branch.clone(), dim());
                    let summary = if a.merged {
                        vec![
                            Span::raw("remove "),
                            worktree,
                            Span::raw(" + branch "),
                            branch,
                        ]
                    } else {
                        vec![
                            Span::raw("remove "),
                            worktree,
                            Span::raw(" (branch "),
                            branch,
                            Span::raw(" kept)"),
                        ]
                    };
                    let mut line = vec![
                        Span::raw("    "),
                        styles::cross_mark(),
                        Span::raw(format!(
                            " {}/{} ({})  ",
                            a.repo.owner,
                            a.repo.name,
                            a.repo.source_label()
                        )),
                    ];
                    line.extend(summary);
                    lines.push(Line::from(line));
                }
            }
        }
        if self.clones > 0 {
            lines.push(Line::from(""));
            for a in &self.actions {
                if a.action_type == ActionType::Clone {
                    lines.push(Line::from(vec![
                        Span::raw(format!(
                            "    + {}/{} ({})  → ",
                            a.repo.owner,
                            a.repo.name,
                            a.repo.source_label()
                        )),
                        Span::styled(a.expected_path.to_string_lossy().into_owned(), dim()),
                    ]));
                }
            }
        }
        if self.dry_run {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  (dry-run mode — no changes will be made)".to_owned(),
                warn(),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  [Enter] Execute  [q] Quit".to_owned(),
            dim(),
        )));
        lines
    }

    /// Go: `renderSummary` (prune results are appended in M5c).
    fn render_summary(&self) -> Vec<Line<'static>> {
        if let Some(err) = &self.load_err {
            return vec![Line::from(vec![
                Span::raw("  "),
                styles::cross_mark(),
                Span::raw(format!(" {err}")),
            ])];
        }

        let mut lines = vec![styles::section_header("Sync Summary")];

        if !self.succeeded.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  Succeeded:".to_owned(),
                success(),
            )));
            for r in &self.succeeded {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    styles::checkmark(),
                    Span::raw(format!(" {:<30} ", r.description)),
                    Span::styled(r.path.clone(), dim()),
                ]));
            }
        }

        if !self.failed.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  Failed:".to_owned(), error())));
            for r in &self.failed {
                lines.push(Line::from(vec![
                    Span::raw("    "),
                    styles::cross_mark(),
                    Span::raw(format!(" {:<30} ", r.description)),
                    Span::styled(r.message.clone(), error()),
                ]));
            }
        }

        // Go: the prune results block inside renderSummary.
        if !self.prune_results.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(""));
            lines.push(styles::section_header("Prune Summary"));
            lines.push(Line::from(""));
            for r in &self.prune_results {
                if r.success {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        styles::checkmark(),
                        Span::raw(format!(" {} ({})", r.name, r.message)),
                    ]));
                } else {
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        styles::cross_mark(),
                        Span::raw(format!(" {} — ", r.name)),
                        Span::styled(r.message.clone(), error()),
                    ]));
                }
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Succeeded:".to_owned(), label()),
            Span::raw(" "),
            Span::styled(self.succeeded.len().to_string(), success()),
            Span::raw(" | "),
            Span::styled("Failed:".to_owned(), label()),
            Span::raw(" "),
            Span::styled(self.failed.len().to_string(), error()),
        ]));

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  [q] Quit".to_owned(), dim())));
        lines
    }

    /// Go: `renderMoveConfirm`.
    fn render_move_confirm(&self) -> Vec<Line<'static>> {
        if self.move_confirm_cursor >= self.move_confirm_indices.len() {
            return Vec::new();
        }
        let action = &self.actions[self.move_confirm_indices[self.move_confirm_cursor]];
        let current = self.move_confirm_cursor + 1;
        let total = self.move_confirm_indices.len();
        vec![
            styles::section_header(&format!("Confirm move {current}/{total}")),
            Line::from(""),
            Line::from(format!(
                "  Move {}/{} ({})?",
                action.repo.owner,
                action.repo.name,
                action.repo.source_label()
            )),
            Line::from(""),
            Line::from(Span::styled(
                action.current_path.to_string_lossy().into_owned(),
                dim(),
            )),
            Line::from(Span::styled(
                format!("→ {}", action.expected_path.to_string_lossy()),
                dim(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "[y] Move  [n] Skip  [a] Move all remaining  [q] Cancel".to_owned(),
                dim(),
            )),
        ]
    }

    /// Go: `renderWorktreeConfirm`.
    fn render_worktree_confirm(&self) -> Vec<Line<'static>> {
        if self.worktree_confirm_cursor >= self.worktree_confirm_indices.len() {
            return Vec::new();
        }
        let action = &self.actions[self.worktree_confirm_indices[self.worktree_confirm_cursor]];
        let current = self.worktree_confirm_cursor + 1;
        let total = self.worktree_confirm_indices.len();

        let branch = Span::styled(action.branch.clone(), success());
        let (fate, command) = if action.merged {
            (
                vec![
                    Span::raw("tracks branch "),
                    branch,
                    Span::raw(", which no longer exists upstream and has been merged.".to_owned()),
                ],
                format!(
                    "git worktree remove {} && git branch -d {}",
                    action.worktree_path.to_string_lossy(),
                    action.branch
                ),
            )
        } else {
            (
                vec![
                    Span::raw("tracks branch "),
                    branch,
                    Span::raw(", which no longer exists upstream.".to_owned()),
                ],
                format!(
                    "git worktree remove {}",
                    action.worktree_path.to_string_lossy()
                ),
            )
        };

        let mut lines = vec![
            styles::section_header(&format!("Stale worktree {current}/{total}")),
            Line::from(""),
            Line::from(vec![
                Span::raw(format!(
                    "  {}/{} ({}): worktree at ",
                    action.repo.owner,
                    action.repo.name,
                    action.repo.source_label()
                )),
                Span::styled(action.worktree_path.to_string_lossy().into_owned(), dim()),
            ]),
        ];
        let mut fate_line = vec![Span::raw("    ")];
        fate_line.extend(fate);
        lines.push(Line::from(fate_line));
        if !action.merged {
            lines.push(Line::from(
                "    The branch has commits not merged by ancestry (squash-merged?) and will be kept.",
            ));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(command, dim())));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "[y] Remove  [n] Skip  [a] Remove all remaining  [q] Cancel".to_owned(),
            dim(),
        )));
        lines
    }

    /// Go: `renderDirtyConfirm`.
    fn render_dirty_confirm(&self) -> Vec<Line<'static>> {
        if self.dirty_confirm_cursor >= self.dirty_confirm_indices.len() {
            return Vec::new();
        }
        let action = &self.actions[self.dirty_confirm_indices[self.dirty_confirm_cursor]];
        let current = self.dirty_confirm_cursor + 1;
        let total = self.dirty_confirm_indices.len();
        vec![
            styles::section_header(&format!("Dirty repo {current}/{total}")),
            Line::from(""),
            Line::from(format!(
                "  {}/{} ({}) has uncommitted changes.",
                action.repo.owner,
                action.repo.name,
                action.repo.source_label()
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("    {}", action.local_path.to_string_lossy()),
                dim(),
            )),
            Line::from(""),
            Line::from(vec![
                Span::raw("  Updating stashes them first (recoverable via "),
                Span::styled("git stash pop".to_owned(), dim()),
                Span::raw(")."),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "[y] Stash & update  [n] Skip  [a] Stash all remaining  [q] Cancel".to_owned(),
                dim(),
            )),
        ]
    }

    /// Go: `renderExecuting` — the completion counter plus the result log.
    fn render_executing(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            styles::section_header(&format!("Executing  {}/{}", self.completed, self.total)),
            Line::from(""),
        ];
        for entry in &self.exec_log {
            lines.push(Line::from(entry.clone()));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  [q] Quit".to_owned(), dim())));
        lines
    }

    /// Go: `renderTrackingFix` — the per-item fix prompt with the previous
    /// results above it.
    fn render_tracking_fix(&self) -> Vec<Line<'static>> {
        if self.tracking_fix_cursor >= self.tracking_fix_items.len() {
            return Vec::new();
        }
        let item = &self.tracking_fix_items[self.tracking_fix_cursor];
        let current = self.tracking_fix_cursor + 1;
        let total = self.tracking_fix_items.len();

        let mut lines = vec![
            styles::section_header(&format!("Fix tracking branch {current}/{total}")),
            Line::from(""),
        ];
        for r in &self.tracking_fix_results {
            lines.push(Line::from(crate::tui::sync::format_action_result(r)));
        }

        let (problem, question, command) = match item.kind {
            super::sync::TrackingFixKind::PushUpstream => (
                format!("{} has no upstream tracking branch.", item.description),
                format!(
                    "Push {} to {} and set upstream?",
                    item.branch, item.remote_name
                ),
                format!("git push -u {} {}", item.remote_name, item.branch),
            ),
            super::sync::TrackingFixKind::StaleUpstream => (
                format!(
                    "{} tracks a remote ref that no longer exists.",
                    item.description
                ),
                format!(
                    "Switch off {}, pull default branch, and delete {} if merged?",
                    item.branch, item.branch
                ),
                format!(
                    "git checkout <default> && git pull --rebase && git branch -d {}",
                    item.branch
                ),
            ),
            super::sync::TrackingFixKind::SetUpstream => (
                format!("{} has no upstream tracking branch.", item.description),
                format!(
                    "Set upstream to {}/{} and pull?",
                    item.remote_name, item.branch
                ),
                format!(
                    "git branch --set-upstream-to={}/{} {}",
                    item.remote_name, item.branch, item.branch
                ),
            ),
        };
        let style_problem = |t: String| Line::from(Span::styled(t, error()));
        let style_ok = |t: String| Line::from(Span::styled(t, success()));
        lines.push(Line::from(""));
        lines.push(style_problem(problem));
        lines.push(Line::from(""));
        lines.push(style_ok(question));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(command, dim())));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "[y] Fix  [n] Skip  [q] Skip all remaining".to_owned(),
            dim(),
        )));
        lines
    }

    /// Go: `renderPruneList`.
    fn render_prune_list(&self) -> Vec<Line<'static>> {
        let mut lines = vec![Line::from("")];
        lines.push(Line::from(format!(
            "  Found {} orphan repos",
            self.orphans.len()
        )));
        lines.push(Line::from(""));
        for o in &self.orphans {
            lines.push(Line::from(vec![
                Span::raw("    "),
                styles::bullet(),
                Span::raw(format!(" {} ", o.name)),
                Span::styled(o.path.to_string_lossy().into_owned(), dim()),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  [y] Remove all  [n] Skip  [i] Interactive".to_owned(),
            dim(),
        )));
        lines
    }

    /// Go: `renderPruneConfirm`.
    fn render_prune_confirm(&self) -> Vec<Line<'static>> {
        if self.prune_cursor >= self.orphans.len() {
            return self.render_prune_done();
        }
        let o = &self.orphans[self.prune_cursor];
        vec![
            Line::from(""),
            Line::from(vec![
                Span::raw("  Remove orphan "),
                Span::styled(o.name.clone(), warn()),
                Span::raw(" ("),
                Span::styled(o.path.to_string_lossy().into_owned(), dim()),
                Span::raw(")?"),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "  [y] Yes  [n] No  [a] Yes to all  [q] Quit".to_owned(),
                dim(),
            )),
        ]
    }

    /// Go: `renderPruneDone`.
    fn render_prune_done(&self) -> Vec<Line<'static>> {
        let mut lines = vec![styles::section_header("Prune Summary")];
        let mut removed = 0;
        let mut failed = 0;
        for r in &self.prune_results {
            if r.success {
                removed += 1;
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    styles::checkmark(),
                    Span::raw(format!(" {} ({})\n", r.name, r.message)),
                ]));
            } else {
                failed += 1;
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    styles::cross_mark(),
                    Span::raw(format!(" {} — ", r.name)),
                    Span::styled(r.message.clone(), error()),
                ]));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Removed:".to_owned(), label()),
            Span::raw(" "),
            Span::styled(removed.to_string(), success()),
            Span::raw(" | "),
            Span::styled("Failed:".to_owned(), label()),
            Span::raw(" "),
            Span::styled(failed.to_string(), error()),
        ]));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  [Enter] Continue".to_owned(),
            dim(),
        )));
        lines
    }
}
