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
            Phase::Summary => self.render_summary(),
            // M5b/M5c views; unreachable from M5a's flow but rendered
            // defensively as the log tail rather than panicking.
            Phase::DirtyConfirm
            | Phase::MoveConfirm
            | Phase::WorktreeConfirm
            | Phase::Executing
            | Phase::TrackingFix
            | Phase::PruneList
            | Phase::PruneConfirm
            | Phase::PruneDone => self.render_exec_log_fallback(),
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

    /// A defensive view for phases whose renderers arrive in M5b/M5c: the
    /// execution log tail plus the standing key hint, so a key path that
    /// reaches an unwired phase stays usable (q always quits).
    fn render_exec_log_fallback(&self) -> Vec<Line<'static>> {
        let mut lines = vec![styles::section_header("Executing")];
        for entry in self.exec_log.iter().rev().take(10) {
            lines.push(Line::from(entry.clone()));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  [q] Quit".to_owned(), dim())));
        lines
    }
}
