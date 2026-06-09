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
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
)

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

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

// trackingFixItem holds the info needed to prompt the user about
// setting a missing upstream tracking branch.
type trackingFixItem struct {
	Description string // e.g. "BatchLabs/lib.go.sender (github)"
	Path        string // local repo path
	RemoteName  string // e.g. "origin"
	Branch      string // e.g. "feat/uj-temporal-rewrite"
}

type trackingFixResultMsg struct {
	result execute.ActionResult
	index  int
}

// ---------------------------------------------------------------------------
// Phases
// ---------------------------------------------------------------------------

type syncPhase int

const (
	syncPhaseLoading syncPhase = iota
	syncPhasePlan
	syncPhaseMoveConfirm
	syncPhaseExecuting
	syncPhaseSummary
	syncPhasePruneList
	syncPhasePruneConfirm
	syncPhasePruneDone
	syncPhaseTrackingFix
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
	skipPull    bool
	concurrency int

	// Config
	ws *config.Workspace

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
	viewport     viewport.Model
	ready        bool
	updates      int
	skippedPulls int
	moves        int
	clones       int

	// Move confirmation phase
	moveConfirmIndices []int
	moveConfirmCursor  int
	skippedMoveIndices map[int]bool

	// Execution phase
	completed   int
	total       int
	execCh      <-chan execute.ActionResult
	execResults []execute.ActionResult
	execLog     []string

	// Summary phase
	succeeded []execute.ActionResult
	failed    []execute.ActionResult

	// Tracking fix phase
	trackingFixItems   []trackingFixItem
	trackingFixCursor  int
	trackingFixResults []execute.ActionResult

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
}

type loadingStep struct {
	label string
	done  bool
}

// NewSyncModel creates a new sync TUI model.
func NewSyncModel(workspaceName string, ws *config.Workspace, dryRun, interactive, doPrune, skipPull bool, concurrency int) SyncModel {
	s := spinner.New()
	s.Spinner = spinner.Dot
	s.Style = SuccessStyle

	steps := []loadingStep{
		{label: "Fetching GitHub repos..."},
		{label: "Fetching Forgejo repos..."},
		{label: "Scanning local repos..."},
	}

	ctx, cancel := context.WithCancel(context.Background())

	return SyncModel{
		phase:        syncPhaseLoading,
		workspace:    workspaceName,
		dryRun:       dryRun,
		interactive:  interactive,
		doPrune:      doPrune,
		skipPull:     skipPull,
		concurrency:  concurrency,
		ws:           ws,
		spinner:      s,
		loadingSteps: steps,
		loadingIdx:   0,
		ctx:          ctx,
		cancel:       cancel,
	}
}

func (m SyncModel) Init() tea.Cmd {
	return tea.Batch(m.spinner.Tick, m.fetchGithubRepos())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

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
		local, err := discover.Discover(m.ws.Root)
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

func (m SyncModel) startExecution(actions []syncplan.Action, initialResults []execute.ActionResult) (tea.Model, tea.Cmd) {
	m.phase = syncPhaseExecuting
	m.total = len(actions) + len(initialResults)
	m.completed = len(initialResults)
	m.execResults = append([]execute.ActionResult(nil), initialResults...)
	m.execLog = nil
	for _, result := range initialResults {
		m.execLog = append(m.execLog, formatActionResult(result))
	}
	m.succeeded = nil
	m.failed = nil

	if len(actions) == 0 {
		return m.finishExecution()
	}

	m.execCh = execute.ExecuteActions(m.ctx, actions, m.dryRun, nil, m.concurrency)
	return m, m.waitForActionResult()
}

func (m SyncModel) buildExecutableActions() []syncplan.Action {
	actions := make([]syncplan.Action, 0, len(m.actions))
	for i, action := range m.actions {
		if action.Type == syncplan.ActionMove && m.skippedMoveIndices[i] {
			continue
		}
		if action.Type == syncplan.ActionUpdate && m.skipPull && action.AlreadyInPlace {
			continue
		}
		actions = append(actions, action)
	}
	return actions
}

func (m SyncModel) buildSkippedPullResults() []execute.ActionResult {
	if !m.skipPull {
		return nil
	}

	results := make([]execute.ActionResult, 0, m.skippedPulls)
	for _, action := range m.actions {
		if action.Type != syncplan.ActionUpdate || !action.AlreadyInPlace {
			continue
		}

		results = append(results, execute.ActionResult{
			Description: fmt.Sprintf("%s/%s (%s)", action.Repo.Owner, action.Repo.Name, action.Repo.SourceLabel()),
			Path:        action.LocalPath,
			Success:     true,
			Message:     "skipped (pull disabled)",
		})
	}
	return results
}

func (m SyncModel) buildSkippedMoveResults() []execute.ActionResult {
	if len(m.skippedMoveIndices) == 0 {
		return nil
	}

	results := make([]execute.ActionResult, 0, len(m.skippedMoveIndices))
	for i, action := range m.actions {
		if action.Type != syncplan.ActionMove || !m.skippedMoveIndices[i] {
			continue
		}

		results = append(results, execute.ActionResult{
			Description: fmt.Sprintf("%s/%s (%s)", action.Repo.Owner, action.Repo.Name, action.Repo.SourceLabel()),
			Path:        action.CurrentPath,
			Success:     true,
			Message:     "skipped (user declined)",
		})
	}
	return results
}

func (m SyncModel) finishExecution() (tea.Model, tea.Cmd) {
	for _, r := range m.execResults {
		if r.Success {
			m.succeeded = append(m.succeeded, r)
		} else {
			m.failed = append(m.failed, r)
		}
	}

	// Detect "no tracking information" failures and offer to fix them.
	for _, r := range m.failed {
		if !execute.IsNoTrackingError(r.Message) {
			continue
		}
		branch, err := execute.GetCurrentBranch(r.Path)
		if err != nil {
			continue
		}
		exists, err := execute.RemoteBranchExists(r.Path, "origin", branch)
		if err != nil || !exists {
			continue
		}
		m.trackingFixItems = append(m.trackingFixItems, trackingFixItem{
			Description: r.Description,
			Path:        r.Path,
			RemoteName:  "origin",
			Branch:      branch,
		})
	}

	if len(m.trackingFixItems) > 0 {
		// Remove tracking-fixable items from the failed list
		// so they don't show in the summary before resolution.
		fixable := make(map[string]bool)
		for _, item := range m.trackingFixItems {
			fixable[item.Path] = true
		}
		var remaining []execute.ActionResult
		for _, r := range m.failed {
			if !fixable[r.Path] {
				remaining = append(remaining, r)
			}
		}
		m.failed = remaining

		m.phase = syncPhaseTrackingFix
		m.trackingFixCursor = 0
		m.trackingFixResults = nil
		return m, nil
	}

	if m.doPrune {
		allRemote := append(m.githubRepos, m.forgejoRepos...)
		m.orphans = prune.FindOrphans(m.localRepos, allRemote, m.ws)
		if len(m.orphans) > 0 {
			m.phase = syncPhasePruneList
			return m, nil
		}
	}

	m.phase = syncPhaseSummary
	return m, tea.Quit
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

func (m SyncModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		if !m.ready {
			m.viewport = viewport.New(msg.Width, msg.Height-2)
			m.ready = true
		} else {
			m.viewport.Width = msg.Width
			m.viewport.Height = msg.Height - 2
		}
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
	case syncGithubReposMsg:
		if msg.err != nil {
			m.loadErr = msg.err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.githubRepos = msg.repos
		m.loadingSteps[0].done = true
		m.loadingIdx = 1
		return m, tea.Batch(m.spinner.Tick, m.fetchForgejoRepos())

	case syncForgejoReposMsg:
		if msg.err != nil {
			m.loadErr = msg.err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.forgejoRepos = msg.repos
		m.loadingSteps[1].done = true
		m.loadingIdx = 2
		return m, tea.Batch(m.spinner.Tick, m.scanLocalRepos())

	case syncLocalReposMsg:
		if msg.err != nil {
			m.loadErr = msg.err
			m.phase = syncPhaseSummary
			return m, tea.Quit
		}
		m.localRepos = msg.local
		m.loadingSteps[2].done = true

		// Dedup and build plan
		githubRepos, forgejoRepos := remote.DedupRepos(m.githubRepos, m.forgejoRepos)
		m.githubRepos = githubRepos
		m.forgejoRepos = forgejoRepos

		m.actions = syncplan.BuildPlan(append(m.githubRepos, m.forgejoRepos...), m.localRepos, m.ws)

		for _, a := range m.actions {
			switch a.Type {
			case syncplan.ActionUpdate:
				if m.skipPull && a.AlreadyInPlace {
					m.skippedPulls++
				} else {
					m.updates++
				}
			case syncplan.ActionMove:
				m.moves++
			case syncplan.ActionClone:
				m.clones++
			}
		}

		m.phase = syncPhasePlan
		m.viewport.SetContent(m.buildPlanContent())
		return m, nil

	// Execution phase messages
	case syncActionResultMsg:
		m.completed++
		m.execResults = append(m.execResults, msg.result)

		m.execLog = append(m.execLog, formatActionResult(msg.result))

		if m.completed >= m.total {
			return m.finishExecution()
		}
		return m, m.waitForActionResult()

	case trackingFixResultMsg:
		m.trackingFixResults = append(m.trackingFixResults, msg.result)
		if msg.result.Success {
			m.succeeded = append(m.succeeded, msg.result)
		} else {
			m.failed = append(m.failed, msg.result)
		}
		m.trackingFixCursor++
		if m.trackingFixCursor >= len(m.trackingFixItems) {
			return m.afterTrackingFix()
		}
		return m, nil
	}

	return m, nil
}

func (m SyncModel) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch m.phase {
	case syncPhasePlan:
		switch msg.String() {
		case "enter":
			if m.interactive && !m.dryRun && m.moves > 0 {
				m.phase = syncPhaseMoveConfirm
				m.moveConfirmIndices = m.moveConfirmIndices[:0]
				for i, action := range m.actions {
					if action.Type == syncplan.ActionMove {
						m.moveConfirmIndices = append(m.moveConfirmIndices, i)
					}
				}
				m.moveConfirmCursor = 0
				m.skippedMoveIndices = make(map[int]bool)
				return m, nil
			}

			return m.startExecution(m.buildExecutableActions(), m.buildSkippedPullResults())

		case "q", "ctrl+c":
			m.cancel()
			return m, tea.Quit

		default:
			var cmd tea.Cmd
			m.viewport, cmd = m.viewport.Update(msg)
			return m, cmd
		}

	case syncPhaseMoveConfirm:
		switch msg.String() {
		case "y", "Y", "enter":
			m.moveConfirmCursor++
		case "n", "N":
			if m.moveConfirmCursor < len(m.moveConfirmIndices) {
				m.skippedMoveIndices[m.moveConfirmIndices[m.moveConfirmCursor]] = true
			}
			m.moveConfirmCursor++
		case "a", "A":
			m.moveConfirmCursor = len(m.moveConfirmIndices)
		case "q", "esc", "ctrl+c":
			m.phase = syncPhasePlan
			return m, nil
		default:
			return m, nil
		}

		if m.moveConfirmCursor >= len(m.moveConfirmIndices) {
			initialResults := append(m.buildSkippedPullResults(), m.buildSkippedMoveResults()...)
			return m.startExecution(m.buildExecutableActions(), initialResults)
		}
		return m, nil

	case syncPhaseTrackingFix:
		switch msg.String() {
		case "y", "Y", "enter":
			item := m.trackingFixItems[m.trackingFixCursor]
			result := execute.SetUpstreamAndPull(item.Path, item.RemoteName, item.Branch)
			result.Description = item.Description
			if result.Success {
				m.succeeded = append(m.succeeded, result)
			} else {
				m.failed = append(m.failed, result)
			}
			m.trackingFixCursor++
			if m.trackingFixCursor >= len(m.trackingFixItems) {
				return m.afterTrackingFix()
			}
			return m, nil
		case "n", "N":
			item := m.trackingFixItems[m.trackingFixCursor]
			m.failed = append(m.failed, execute.ActionResult{
				Description: item.Description,
				Path:        item.Path,
				Success:     false,
				Message:     "no tracking branch (user declined fix)",
			})
			m.trackingFixCursor++
			if m.trackingFixCursor >= len(m.trackingFixItems) {
				return m.afterTrackingFix()
			}
			return m, nil
		case "q", "ctrl+c":
			for i := m.trackingFixCursor; i < len(m.trackingFixItems); i++ {
				item := m.trackingFixItems[i]
				m.failed = append(m.failed, execute.ActionResult{
					Description: item.Description,
					Path:        item.Path,
					Success:     false,
					Message:     "no tracking branch (skipped)",
				})
			}
			return m.afterTrackingFix()
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
	case syncPhaseMoveConfirm:
		return m.renderMoveConfirm()
	case syncPhaseExecuting:
		return m.renderExecuting()
	case syncPhaseTrackingFix:
		return m.renderTrackingFix()
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
	if !m.ready {
		return ""
	}
	return m.viewport.View()
}

func (m SyncModel) renderMoveConfirm() string {
	if m.moveConfirmCursor >= len(m.moveConfirmIndices) {
		return ""
	}

	action := m.actions[m.moveConfirmIndices[m.moveConfirmCursor]]
	current := m.moveConfirmCursor + 1
	total := len(m.moveConfirmIndices)

	return fmt.Sprintf(
		"%s\n\n  Move %s/%s (%s)?\n\n    %s\n    %s\n\n  %s\n",
		SectionHeader(fmt.Sprintf("Confirm move %d/%d", current, total)),
		action.Repo.Owner,
		action.Repo.Name,
		action.Repo.SourceLabel(),
		DimStyle.Render(action.CurrentPath),
		DimStyle.Render("→ "+action.ExpectedPath),
		DimStyle.Render("[y] Move  [n] Skip  [a] Move all remaining  [q] Cancel"),
	)
}

func (m SyncModel) buildPlanContent() string {
	var sb strings.Builder

	sb.WriteString(SectionHeader("Plan"))
	sb.WriteString("\n\n  ")
	sb.WriteString(CountDisplay(m.updates, m.moves, m.clones))
	if m.skippedPulls > 0 {
		if m.updates > 0 || m.moves > 0 || m.clones > 0 {
			sb.WriteString(", ")
		}
		sb.WriteString(fmt.Sprintf("%s %s",
			SuccessStyle.Render(fmt.Sprintf("%d", m.skippedPulls)),
			LabelStyle.Render("pulls skipped"),
		))
	}
	sb.WriteString("\n")

	if m.updates > 0 {
		sb.WriteString(fmt.Sprintf("\n    %s\n", DimStyle.Render(fmt.Sprintf("%d repos will get a git pull --rebase", m.updates))))
	}
	if m.skippedPulls > 0 {
		sb.WriteString(fmt.Sprintf("    %s %s\n", Checkmark(), DimStyle.Render(fmt.Sprintf("%d repos are already in the expected location (pull skipped)", m.skippedPulls))))
	}
	if m.moves > 0 {
		sb.WriteString("\n")
		for _, a := range m.actions {
			if a.Type == syncplan.ActionMove {
				sb.WriteString(fmt.Sprintf("    %s %s/%s (%s)  %s → %s\n",
					Arrow(), a.Repo.Owner, a.Repo.Name, a.Repo.SourceLabel(),
					DimStyle.Render(a.CurrentPath),
					DimStyle.Render(a.ExpectedPath)))
			}
		}
	}
	if m.clones > 0 {
		sb.WriteString("\n")
		for _, a := range m.actions {
			if a.Type == syncplan.ActionClone {
				sb.WriteString(fmt.Sprintf("    + %s/%s (%s)  → %s\n",
					a.Repo.Owner, a.Repo.Name, a.Repo.SourceLabel(),
					DimStyle.Render(a.ExpectedPath)))
			}
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

	sb.WriteString(SectionHeader(fmt.Sprintf("Executing  %d/%d", m.completed, m.total)))
	sb.WriteString("\n\n")

	for _, line := range m.execLog {
		sb.WriteString(line)
		sb.WriteString("\n")
	}

	sb.WriteString(DimStyle.Render("\n  [q] Quit"))
	sb.WriteString("\n")

	return sb.String()
}

func (m SyncModel) renderTrackingFix() string {
	if m.trackingFixCursor >= len(m.trackingFixItems) {
		return ""
	}

	item := m.trackingFixItems[m.trackingFixCursor]
	current := m.trackingFixCursor + 1
	total := len(m.trackingFixItems)

	// Show results of previous fixes
	var prevResults strings.Builder
	for _, r := range m.trackingFixResults {
		prevResults.WriteString(formatActionResult(r))
		prevResults.WriteString("\n")
	}

	return fmt.Sprintf(
		"%s\n\n  %s\n\n    %s\n\n    %s\n\n  %s\n",
		SectionHeader(fmt.Sprintf("Fix tracking branch %d/%d", current, total)),
		fmt.Sprintf("%s has no upstream tracking branch.", ErrorStyle.Render(item.Description)),
		fmt.Sprintf("Set upstream to %s and pull?", SuccessStyle.Render(fmt.Sprintf("%s/%s", item.RemoteName, item.Branch))),
		DimStyle.Render(fmt.Sprintf("git branch --set-upstream-to=%s/%s %s", item.RemoteName, item.Branch, item.Branch)),
		DimStyle.Render("[y] Fix and pull  [n] Skip  [q] Skip all remaining"),
	)
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
			sb.WriteString(fmt.Sprintf("    %s %-30s %s\n", Checkmark(), r.Description, DimStyle.Render(r.Path)))
		}
	}

	if len(m.failed) > 0 {
		sb.WriteString("\n  ")
		sb.WriteString(ErrorStyle.Render("Failed:"))
		sb.WriteString("\n")
		for _, r := range m.failed {
			sb.WriteString(fmt.Sprintf("    %s %-30s %s\n", CrossMark(), r.Description, ErrorStyle.Render(r.Message)))
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

// formatActionResult formats a single action result as a log line.
func formatActionResult(r execute.ActionResult) string {
	switch {
	case r.Success:
		return fmt.Sprintf("  %s %-30s %s", Checkmark(), r.Description, DimStyle.Render(r.Path))
	default:
		return fmt.Sprintf("  %s %-30s %s", CrossMark(), r.Description, ErrorStyle.Render(r.Message))
	}
}

func (m SyncModel) afterTrackingFix() (tea.Model, tea.Cmd) {
	if m.doPrune {
		allRemote := append(m.githubRepos, m.forgejoRepos...)
		m.orphans = prune.FindOrphans(m.localRepos, allRemote, m.ws)
		if len(m.orphans) > 0 {
			m.phase = syncPhasePruneList
			return m, nil
		}
	}
	m.phase = syncPhaseSummary
	return m, tea.Quit
}

// HasFailures returns true if any action or prune failed.
func (m SyncModel) HasFailures() bool {
	return len(m.failed) > 0
}
