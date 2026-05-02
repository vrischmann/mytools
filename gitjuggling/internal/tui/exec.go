package tui

import (
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/bubbles/spinner"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

// ExecResultMsg is sent when a single repo command completes.
type ExecResultMsg struct {
	Path    string
	Success bool
	Stdout  string
	Stderr  string
	Err     error
}

// ExecDoneMsg is sent when all commands have completed.
type ExecDoneMsg struct{}

// ExecDiscoverMsg is sent with the list of discovered repo paths.
type ExecDiscoverMsg struct {
	Paths []string
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

type execPhase int

const (
	execPhaseDiscovering execPhase = iota
	execPhaseRunning
	execPhaseDone
)

// ExecModel is the Bubbletea model for the exec command.
type ExecModel struct {
	phase     execPhase
	verbose   bool
	total     int
	completed int

	// Discovering phase
	spinner SpinnerModel

	// Running phase
	progress  ProgressModel
	repoPaths []string
	gitArgs   []string

	// Results
	results []ExecResultMsg

	// Timing
	startTime time.Time
	elapsed   time.Duration

	// Output channel from goroutines
	resultsCh <-chan ExecResultMsg

	width  int
	height int
}

// NewExecModel creates a new exec TUI model.
func NewExecModel(gitArgs []string, verbose bool, resultsCh <-chan ExecResultMsg) ExecModel {
	return ExecModel{
		phase:     execPhaseDiscovering,
		verbose:   verbose,
		gitArgs:   gitArgs,
		spinner:   NewSpinnerModel("Scanning repos..."),
		progress:  NewProgressModel(),
		resultsCh: resultsCh,
	}
}

func (m ExecModel) Init() tea.Cmd {
	return m.spinner.Spinner.Tick
}

func (m ExecModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		return m, nil

	case tea.KeyMsg:
		switch msg.String() {
		case "q", "ctrl+c":
			return m, tea.Quit
		}

	case ExecDiscoverMsg:
		m.phase = execPhaseRunning
		m.total = len(msg.Paths)
		m.repoPaths = msg.Paths
		m.startTime = time.Now()
		m.progress.SetProgress(0, fmt.Sprintf("0/%d", m.total))
		return m, m.waitForResult()

	case ExecResultMsg:
		m.completed++
		m.results = append(m.results, msg)

		ratio := 0.0
		if m.total > 0 {
			ratio = float64(m.completed) / float64(m.total)
		}
		repoName := ""
		if msg.Path != "" {
			parts := strings.Split(msg.Path, "/")
			repoName = parts[len(parts)-1]
		}
		m.progress.SetProgress(ratio, fmt.Sprintf("%d/%d  %s", m.completed, m.total, repoName))

		if m.completed >= m.total {
			m.phase = execPhaseDone
			m.elapsed = time.Since(m.startTime)
			return m, tea.Quit
		}
		return m, m.waitForResult()

	case spinner.TickMsg:
		if m.phase == execPhaseDiscovering {
			var cmd tea.Cmd
			m.spinner, cmd = m.spinner.Update(msg)
			return m, cmd
		}
	}

	// Update spinner/progress sub-models
	if m.phase == execPhaseDiscovering {
		var cmd tea.Cmd
		m.spinner, cmd = m.spinner.Update(msg)
		return m, cmd
	}

	if m.phase == execPhaseRunning {
		var cmd tea.Cmd
		m.progress, cmd = m.progress.Update(msg)
		return m, cmd
	}

	return m, nil
}

func (m ExecModel) View() string {
	switch m.phase {
	case execPhaseDiscovering:
		return m.spinner.View()

	case execPhaseRunning:
		return fmt.Sprintf("  Running git %s\n%s", strings.Join(m.gitArgs, " "), m.progress.View())

	case execPhaseDone:
		return m.renderSummary()

	default:
		return ""
	}
}

func (m ExecModel) renderSummary() string {
	var sb strings.Builder

	var succeeded, failed []ExecResultMsg
	for _, r := range m.results {
		if r.Success {
			succeeded = append(succeeded, r)
		} else {
			failed = append(failed, r)
		}
	}

	// Verbose: show all output
	if m.verbose && len(succeeded) > 0 {
		sb.WriteString(SectionHeader("Output"))
		sb.WriteString("\n\n")
		for _, item := range succeeded {
			sb.WriteString(SuccessStyle.Render(item.Path))
			sb.WriteString("\n")
			if item.Stdout != "" {
				sb.WriteString(StdoutStyle.Render(item.Stdout))
				sb.WriteString("\n")
			}
			if item.Stderr != "" {
				sb.WriteString(StderrStyle.Render(item.Stderr))
				sb.WriteString("\n")
			}
			sb.WriteString("\n")
		}
	}

	// Always show failures
	if len(failed) > 0 {
		if !m.verbose {
			sb.WriteString("\n")
		}
		sb.WriteString(SectionHeader("Failed Items"))
		sb.WriteString("\n\n")
		for _, item := range failed {
			sb.WriteString(SuccessStyle.Render(item.Path))
			sb.WriteString("\n")
			if item.Stdout != "" {
				sb.WriteString(StdoutStyle.Render(item.Stdout))
				sb.WriteString("\n")
			}
			if item.Stderr != "" {
				sb.WriteString(StderrStyle.Render(item.Stderr))
				sb.WriteString("\n")
			}
			if item.Err != nil {
				sb.WriteString(fmt.Sprintf("error: %v\n", item.Err))
			}
			sb.WriteString("\n")
		}
	}

	// Summary
	sb.WriteString(SectionHeader("Summary"))
	sb.WriteString("\n\n")
	sb.WriteString(fmt.Sprintf("  %s %s\n", LabelStyle.Render("Succeeded:"), SuccessStyle.Render(fmt.Sprintf("%d", len(succeeded)))))
	sb.WriteString(fmt.Sprintf("  %s %s\n", LabelStyle.Render("Failed:   "), ErrorStyle.Render(fmt.Sprintf("%d", len(failed)))))
	sb.WriteString(fmt.Sprintf("  %s %s\n", LabelStyle.Render("Time:     "), BoldStyle.Render(fmt.Sprintf("%.2fs", m.elapsed.Seconds()))))

	return sb.String()
}

// waitForResult blocks until a result is available on the channel, then sends it as a tea.Msg.
func (m ExecModel) waitForResult() tea.Cmd {
	return func() tea.Msg {
		r, ok := <-m.resultsCh
		if !ok {
			return ExecDoneMsg{}
		}
		return r
	}
}

// HasFailures returns true if any command failed.
func (m ExecModel) HasFailures() bool {
	for _, r := range m.results {
		if !r.Success {
			return true
		}
	}
	return false
}

// Suppress unused import warnings
var _ = lipgloss.Color("0")
