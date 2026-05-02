package tui

import (
	"context"
	"fmt"
	"strings"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/execute"
	"dev.rischmann.fr/mytools/gitjuggling/internal/prune"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
	"github.com/charmbracelet/bubbles/spinner"
	tea "github.com/charmbracelet/bubbletea"
)

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

type syncConfigLoadedMsg struct {
	cfg *config.Config
	err error
}

type syncGithubReposMsg struct {
	repos []*remote.RemoteRepo
	err   error
}

type syncForgejoReposMsg struct {
	repos []*remote.RemoteRepo
	err   error
}

type syncLocalReposMsg struct {
	local *discover.LocalRepos
	err   error
}

type syncActionResultMsg struct {
	result execute.ActionResult
}

type syncPruneResultMsg struct {
	result *prune.PruneResult
}

type syncErrMsg struct {
	err error
}

// ---------------------------------------------------------------------------
// Phases
// ---------------------------------------------------------------------------

type syncPhase int

const (
	syncPhaseLoading syncPhase = iota
	syncPhasePlan
	syncPhaseExecuting
	syncPhaseSummary
	syncPhasePruneList
	syncPhasePruneConfirm
	syncPhasePruneDone
)

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

// SyncModel is the Bubbletea model for the sync command.
type SyncModel struct {
	phase     syncPhase
	workspace string

	// Options
	dryRun      bool
	interactive bool
	doPrune     bool
	concurrency int

	// Config
	cfg *config.Config
	ws  *config.Workspace

	// Loading phase
	spinner      spinner.Model
	loadingSteps []loadingStep
	loadingIdx   int

	// Data from loading
	githubRepos  []*remote.RemoteRepo
	forgejoRepos []*remote.RemoteRepo
	localRepos   *discover.LocalRepos
	actions      []syncplan.Action

	// Plan phase
	updates int
	moves   int
	clones  int

	// Execution phase
	progress    ProgressModel
	completed   int
	total       int
	execCh      <-chan execute.ActionResult
	execResults []execute.ActionResult

	// Summary phase
	succeeded []execute.ActionResult
	failed    []execute.ActionResult

	// Prune
	orphans      []*prune.OrphanRepo
	pruneResults []*prune.PruneResult
	pruneCursor  int
	pruneYesAll  bool

	// Error
	loadErr error

	width  int
	height int
	ctx    context.Context
	cancel context.CancelFunc

	configPath string
}

type loadingStep struct {
	label string
	done  bool
}

// NewSyncModel creates a new sync TUI model.
func NewSyncModel(workspace string, configPath string, dryRun, interactive, doPrune bool, concurrency int) SyncModel {
	s := spinner.New()
	s.Spinner = spinner.Dot
	s.Style = SuccessStyle

	steps := []loadingStep{
		{label: "Loading config..."},
		{label: "Fetching GitHub repos..."},
		{label: "Fetching Forgejo repos..."},
		{label: "Scanning local repos..."},
	}

	ctx, cancel := context.WithCancel(context.Background())

	return SyncModel{
		phase:        syncPhaseLoading,
		workspace:    workspace,
		dryRun:       dryRun,
		interactive:  interactive,
		doPrune:      doPrune,
		concurrency:  concurrency,
		spinner:      s,
		loadingSteps: steps,
		loadingIdx:   0,
		progress:     NewProgressModel(),
		ctx:          ctx,
		cancel:       cancel,
		configPath:   configPath,
	}
}

func (m SyncModel) Init() tea.Cmd {
	return tea.Batch(m.spinner.Tick, m.loadConfig())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

func (m SyncModel) loadConfig() tea.Cmd {
	return func() tea.Msg {
		var cfg *config.Config
		var err error
		if m.configPath != "" {
			cfg, err = config.LoadFrom(m.configPath)
		} else {
			cfg, err = config.LoadDefault()
		}
		return syncConfigLoadedMsg{cfg: cfg, err: err}
	}
}

func (m SyncModel) fetchGithubRepos() tea.Cmd {
	return func() tea.Msg {
		if len(m.ws.GitHubOwners) == 0 {
			return syncGithubReposMsg{}
		}
		repos, err := remote.FetchGitHubRepos(m.ws.GitHubOwners)
		return syncGithubReposMsg{repos: repos, err: err}
	}
}

func (m SyncModel) fetchForgejoRepos() tea.Cmd {
	return func() tea.Msg {
		if m.ws.ForgejoURL == "" || m.ws.ForgejoUser == "" || m.ws.ForgejoToken == "" {
			return syncForgejoReposMsg{}
		}
		repos, err := remote.FetchForgejoRepos(m.ws.ForgejoURL, m.ws.ForgejoUser, m.ws.ForgejoToken)
		return syncForgejoReposMsg{repos: repos, err: err}
	}
}

func (m SyncModel) scanLocalRepos() tea.Cmd {
	return func() tea.Msg {
		local, err := discover.Discover(m.ws.LocalScanRoot())
		return syncLocalReposMsg{local: local, err: err}
	}
}

func (m SyncModel) waitForActionResult() tea.Cmd {
	return func() tea.Msg {
		r, ok := <-m.execCh
		if !ok {
			return nil
		}
		return syncActionResultMsg{result: r}
	}
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

func (m SyncModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil

	case tea.KeyMsg:
		return m.handleKey(msg)

	case spinner.TickMsg:
		if m.phase == syncPhaseLoading {
			var cmd tea.Cmd
			m.spinner, cmd = m.spinner.Update(msg)
			return m, cmd
		}
		return m, nil

	// Loading phase messages
	case syncConfigLoadedMsg:
		if msg.err != nil {
			m.loadErr = msg.err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.cfg = msg.cfg
		ws, err := m.cfg.GetWorkspace(m.workspace)
		if err != nil {
			m.loadErr = err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.ws = ws
		m.loadingSteps[0].done = true
		m.loadingIdx = 1
		return m, tea.Batch(m.spinner.Tick, m.fetchGithubRepos())

	case syncGithubReposMsg:
		if msg.err != nil {
			m.loadErr = msg.err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.githubRepos = msg.repos
		m.loadingSteps[1].done = true
		m.loadingIdx = 2
		return m, tea.Batch(m.spinner.Tick, m.fetchForgejoRepos())

	case syncForgejoReposMsg:
		if msg.err != nil {
			m.loadErr = msg.err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.forgejoRepos = msg.repos
		m.loadingSteps[2].done = true
		m.loadingIdx = 3
		return m, tea.Batch(m.spinner.Tick, m.scanLocalRepos())

	case syncLocalReposMsg:
		if msg.err != nil {
			m.loadErr = msg.err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.localRepos = msg.local
		m.loadingSteps[3].done = true

		// Dedup and build plan
		githubRepos, forgejoRepos := remote.DedupRepos(m.githubRepos, m.forgejoRepos)
		m.githubRepos = githubRepos
		m.forgejoRepos = forgejoRepos

		m.actions = syncplan.BuildPlan(append(m.githubRepos, m.forgejoRepos...), m.localRepos, m.ws)

		for _, a := range m.actions {
			switch a.Type {
			case syncplan.ActionUpdate:
				m.updates++
			case syncplan.ActionMove:
				m.moves++
			case syncplan.ActionClone:
				m.clones++
			}
		}

		m.phase = syncPhasePlan
		return m, nil

	// Execution phase messages
	case syncActionResultMsg:
		m.completed++
		m.execResults = append(m.execResults, msg.result)

		ratio := 0.0
		if m.total > 0 {
			ratio = float64(m.completed) / float64(m.total)
		}
		m.progress.SetProgress(ratio, msg.result.Description)

		if m.completed >= m.total {
			// Partition results
			for _, r := range m.execResults {
				if r.Success {
					m.succeeded = append(m.succeeded, r)
				} else {
					m.failed = append(m.failed, r)
				}
			}

			if m.doPrune {
				allRemote := append(m.githubRepos, m.forgejoRepos...)
				m.orphans = prune.FindOrphans(m.localRepos, allRemote)
				if len(m.orphans) > 0 {
					m.phase = syncPhasePruneList
					return m, nil
				}
			}

			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		return m, m.waitForActionResult()
	}

	return m, nil
}

func (m SyncModel) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch m.phase {
	case syncPhasePlan:
		switch msg.String() {
		case "enter":
			// Execute the plan
			m.phase = syncPhaseExecuting
			m.total = len(m.actions)
			m.completed = 0

			var confirmFn execute.ConfirmFunc
			if m.interactive {
				// In TUI mode we skip interactive confirm for now (could add later)
				confirmFn = nil
			}

			m.execCh = execute.ExecuteActions(m.ctx, m.actions, m.dryRun, confirmFn, m.concurrency)
			m.progress.SetProgress(0, "")
			return m, tea.Batch(m.waitForActionResult())

		case "q", "ctrl+c":
			m.cancel()
			return m, tea.Quit
		}

	case syncPhaseExecuting:
		if msg.String() == "q" || msg.String() == "ctrl+c" {
			m.cancel()
			return m, tea.Quit
		}

	case syncPhasePruneList:
		switch msg.String() {
		case "y":
			// Remove all
			m.pruneYesAll = true
			m.phase = syncPhasePruneConfirm
			m.pruneResults = prune.PruneOrphans(m.orphans, m.dryRun, nil)
			m.phase = syncPhasePruneDone
			return m, nil
		case "n", "q":
			m.phase = syncPhaseSummary
			return m, tea.Quit
		case "i":
			m.phase = syncPhasePruneConfirm
			m.pruneCursor = 0
			return m, nil
		}

	case syncPhasePruneConfirm:
		switch msg.String() {
		case "y":
			if m.pruneCursor < len(m.orphans) {
				orphan := m.orphans[m.pruneCursor]
				result := prune.PruneOrphans([]*prune.OrphanRepo{orphan}, m.dryRun, nil)
				m.pruneResults = append(m.pruneResults, result...)
				m.pruneCursor++
			}
			if m.pruneCursor >= len(m.orphans) {
				m.phase = syncPhasePruneDone
			}
			return m, nil
		case "n":
			m.pruneCursor++
			if m.pruneCursor >= len(m.orphans) {
				m.phase = syncPhasePruneDone
			}
			return m, nil
		case "a":
			// Yes to all remaining
			remaining := m.orphans[m.pruneCursor:]
			results := prune.PruneOrphans(remaining, m.dryRun, nil)
			m.pruneResults = append(m.pruneResults, results...)
			m.phase = syncPhasePruneDone
			return m, nil
		case "q":
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}

	case syncPhasePruneDone:
		if msg.String() == "q" || msg.String() == "enter" {
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}

	case syncPhaseSummary:
		if msg.String() == "q" || msg.String() == "enter" {
			return m, tea.Quit
		}
	}

	return m, nil
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

func (m SyncModel) View() string {
	switch m.phase {
	case syncPhaseLoading:
		return m.renderLoading()
	case syncPhasePlan:
		return m.renderPlan()
	case syncPhaseExecuting:
		return m.renderExecuting()
	case syncPhaseSummary:
		return m.renderSummary()
	case syncPhasePruneList:
		return m.renderPruneList()
	case syncPhasePruneConfirm:
		return m.renderPruneConfirm()
	case syncPhasePruneDone:
		return m.renderPruneDone()
	default:
		return ""
	}
}

func (m SyncModel) renderLoading() string {
	var sb strings.Builder

	wsName := m.workspace
	if wsName == "" {
		wsName = "default"
	}
	sb.WriteString(SectionHeader(fmt.Sprintf("Syncing workspace: %s", wsName)))
	sb.WriteString("\n\n")

	for i, step := range m.loadingSteps {
		switch {
		case step.done:
			sb.WriteString(fmt.Sprintf("  %s %s\n", Checkmark(), step.label))
		case i == m.loadingIdx:
			sb.WriteString(fmt.Sprintf("  %s %s %s\n", Bullet(), step.label, m.spinner.View()))
		default:
			sb.WriteString(fmt.Sprintf("  %s %s\n", PendingBullet(), step.label))
		}
	}

	return sb.String()
}

func (m SyncModel) renderPlan() string {
	var sb strings.Builder

	sb.WriteString(SectionHeader("Plan"))
	sb.WriteString("\n\n  ")
	sb.WriteString(CountDisplay(m.updates, m.moves, m.clones))
	sb.WriteString("\n")

	// Group actions by type
	for _, a := range m.actions {
		switch a.Type {
		case syncplan.ActionUpdate:
			sb.WriteString(fmt.Sprintf("\n    %s %s/%s\n", Checkmark(), a.Repo.Owner, a.Repo.Name))
		}
	}
	for _, a := range m.actions {
		switch a.Type {
		case syncplan.ActionMove:
			sb.WriteString(fmt.Sprintf("\n    %s %s/%s  %s → %s\n",
				Arrow(), a.Repo.Owner, a.Repo.Name,
				DimStyle.Render(a.CurrentPath),
				DimStyle.Render(a.ExpectedPath)))
		}
	}
	for _, a := range m.actions {
		switch a.Type {
		case syncplan.ActionClone:
			sb.WriteString(fmt.Sprintf("\n    + %s/%s  → %s\n",
				a.Repo.Owner, a.Repo.Name,
				DimStyle.Render(a.ExpectedPath)))
		}
	}

	if m.dryRun {
		sb.WriteString("\n  ")
		sb.WriteString(WarnStyle.Render("(dry-run mode — no changes will be made)"))
		sb.WriteString("\n")
	}

	sb.WriteString("\n  ")
	sb.WriteString(DimStyle.Render("[Enter] Execute  [q] Quit"))
	sb.WriteString("\n")

	return sb.String()
}

func (m SyncModel) renderExecuting() string {
	var sb strings.Builder
	sb.WriteString(SectionHeader("Executing"))
	sb.WriteString("\n")
	sb.WriteString(m.progress.View())
	return sb.String()
}

func (m SyncModel) renderSummary() string {
	if m.loadErr != nil {
		return fmt.Sprintf("  %s %v\n", CrossMark(), m.loadErr)
	}

	var sb strings.Builder

	sb.WriteString(SectionHeader("Sync Summary"))
	sb.WriteString("\n")

	if len(m.succeeded) > 0 {
		sb.WriteString("\n  ")
		sb.WriteString(SuccessStyle.Render("Succeeded:"))
		sb.WriteString("\n")
		for _, r := range m.succeeded {
			sb.WriteString(fmt.Sprintf("    %s %s", Checkmark(), r.Description))
			if r.Message != "" && r.Message != "updated" && r.Message != "cloned" && r.Message != "moved" {
				sb.WriteString(DimStyle.Render(fmt.Sprintf(" (%s)", r.Message)))
			}
			sb.WriteString("\n")
		}
	}

	if len(m.failed) > 0 {
		sb.WriteString("\n  ")
		sb.WriteString(ErrorStyle.Render("Failed:"))
		sb.WriteString("\n")
		for _, r := range m.failed {
			sb.WriteString(fmt.Sprintf("    %s %s\n", CrossMark(), r.Description))
			sb.WriteString(fmt.Sprintf("      %s\n", ErrorStyle.Render(r.Message)))
		}
	}

	// Prune results
	if len(m.pruneResults) > 0 {
		sb.WriteString("\n  ")
		sb.WriteString(SectionHeader("Prune Summary"))
		sb.WriteString("\n")
		for _, r := range m.pruneResults {
			if r.Success {
				sb.WriteString(fmt.Sprintf("    %s %s (%s)\n", Checkmark(), r.Name, r.Message))
			} else {
				sb.WriteString(fmt.Sprintf("    %s %s — %s\n", CrossMark(), r.Name, ErrorStyle.Render(r.Message)))
			}
		}
	}

	sb.WriteString(fmt.Sprintf("\n  %s %s | %s %s\n",
		LabelStyle.Render("Succeeded:"), SuccessStyle.Render(fmt.Sprintf("%d", len(m.succeeded))),
		LabelStyle.Render("Failed:"), ErrorStyle.Render(fmt.Sprintf("%d", len(m.failed))),
	))

	sb.WriteString("\n  ")
	sb.WriteString(DimStyle.Render("[q] Quit"))
	sb.WriteString("\n")

	return sb.String()
}

func (m SyncModel) renderPruneList() string {
	var sb strings.Builder

	sb.WriteString(fmt.Sprintf("\n  Found %s orphan repos\n\n",
		WarnStyle.Render(fmt.Sprintf("%d", len(m.orphans))),
	))

	for _, o := range m.orphans {
		sb.WriteString(fmt.Sprintf("    %s %s (%s)\n", Bullet(), o.Name, DimStyle.Render(o.Path)))
	}

	sb.WriteString("\n  ")
	sb.WriteString(DimStyle.Render("[y] Remove all  [n] Skip  [i] Interactive"))
	sb.WriteString("\n")

	return sb.String()
}

func (m SyncModel) renderPruneConfirm() string {
	if m.pruneCursor >= len(m.orphans) {
		return m.renderPruneDone()
	}

	o := m.orphans[m.pruneCursor]
	return fmt.Sprintf(
		"\n  Remove orphan %s (%s)?\n\n  %s\n",
		WarnStyle.Render(o.Name),
		DimStyle.Render(o.Path),
		DimStyle.Render("[y] Yes  [n] No  [a] Yes to all  [q] Quit"),
	)
}

func (m SyncModel) renderPruneDone() string {
	var sb strings.Builder
	sb.WriteString(SectionHeader("Prune Summary"))
	sb.WriteString("\n")

	var removed, failed []*prune.PruneResult
	for _, r := range m.pruneResults {
		if r.Success {
			removed = append(removed, r)
		} else {
			failed = append(failed, r)
		}
	}

	for _, r := range removed {
		sb.WriteString(fmt.Sprintf("  %s %s (%s)\n", Checkmark(), r.Name, r.Message))
	}
	for _, r := range failed {
		sb.WriteString(fmt.Sprintf("  %s %s — %s\n", CrossMark(), r.Name, ErrorStyle.Render(r.Message)))
	}

	sb.WriteString(fmt.Sprintf("\n  %s %s | %s %s\n",
		LabelStyle.Render("Removed:"), SuccessStyle.Render(fmt.Sprintf("%d", len(removed))),
		LabelStyle.Render("Failed:"), ErrorStyle.Render(fmt.Sprintf("%d", len(failed))),
	))

	sb.WriteString("\n  ")
	sb.WriteString(DimStyle.Render("[Enter] Continue"))
	sb.WriteString("\n")

	return sb.String()
}

// HasFailures returns true if any action or prune failed.
func (m SyncModel) HasFailures() bool {
	return len(m.failed) > 0
}
