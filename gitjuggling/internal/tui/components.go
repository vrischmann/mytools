package tui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/progress"
	"github.com/charmbracelet/bubbles/spinner"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ---------------------------------------------------------------------------
// ConfirmModel — Yes/No confirmation dialog
// ---------------------------------------------------------------------------

// ConfirmResultMsg is sent when the user makes a choice.
type ConfirmResultMsg struct {
	Confirmed bool
	ID        string // optional identifier to correlate with caller
}

// ConfirmModel is a Bubbletea model for yes/no confirmation.
type ConfirmModel struct {
	Prompt   string
	ID       string
	quitting bool
}

// NewConfirmModel creates a new confirmation dialog model.
func NewConfirmModel(prompt, id string) ConfirmModel {
	return ConfirmModel{
		Prompt: prompt,
		ID:     id,
	}
}

func (m ConfirmModel) Init() tea.Cmd {
	return nil
}

func (m ConfirmModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "y", "Y":
			m.quitting = true
			return m, tea.Quit
		case "n", "N", "q", "esc":
			m.quitting = false
			return m, tea.Quit
		case "enter":
			m.quitting = true
			return m, tea.Quit
		case "ctrl+c":
			m.quitting = false
			return m, tea.Quit
		}
	}
	return m, nil
}

func (m ConfirmModel) View() string {
	if m.quitting {
		return ""
	}
	return fmt.Sprintf("%s (Y/n) ", m.Prompt)
}

// Result returns the confirmation result after the model has quit.
func (m ConfirmModel) Result() ConfirmResultMsg {
	return ConfirmResultMsg{
		Confirmed: m.quitting,
		ID:        m.ID,
	}
}

// ---------------------------------------------------------------------------
// ProgressModel — progress bar with status message
// ---------------------------------------------------------------------------

// ProgressModel wraps a progress bar with a message.
type ProgressModel struct {
	Progress progress.Model
	Message  string
	Width    int
}

// NewProgressModel creates a new progress model.
func NewProgressModel() ProgressModel {
	p := progress.New(
		progress.WithDefaultGradient(),
		progress.WithWidth(40),
		progress.WithSolidFill("█"),
	)
	p.Full = '█'
	p.Empty = '░'
	return ProgressModel{
		Progress: p,
	}
}

func (m ProgressModel) Init() tea.Cmd {
	return nil
}

func (m ProgressModel) Update(msg tea.Msg) (ProgressModel, tea.Cmd) {
	model, cmd := m.Progress.Update(msg)
	if p, ok := model.(progress.Model); ok {
		m.Progress = p
	}
	return m, cmd
}

func (m ProgressModel) View() string {
	bar := m.Progress.View()
	return fmt.Sprintf("  %s %s", bar, m.Message)
}

// SetProgress updates the progress ratio and message.
func (m *ProgressModel) SetProgress(ratio float64, message string) {
	m.Message = message
	m.Progress.SetPercent(ratio)
}

// ---------------------------------------------------------------------------
// SpinnerModel — spinner with a message
// ---------------------------------------------------------------------------

// SpinnerModel wraps a spinner with a status message.
type SpinnerModel struct {
	Spinner spinner.Model
	Message string
}

// NewSpinnerModel creates a new spinner model.
func NewSpinnerModel(message string) SpinnerModel {
	s := spinner.New()
	s.Spinner = spinner.Dot
	s.Style = lipgloss.NewStyle().Foreground(lipgloss.Color("14")) // cyan
	return SpinnerModel{
		Spinner: s,
		Message: message,
	}
}

func (m SpinnerModel) Init() tea.Cmd {
	return m.Spinner.Tick
}

func (m SpinnerModel) Update(msg tea.Msg) (SpinnerModel, tea.Cmd) {
	var cmd tea.Cmd
	m.Spinner, cmd = m.Spinner.Update(msg)
	return m, cmd
}

func (m SpinnerModel) View() string {
	return fmt.Sprintf("  %s %s", m.Spinner.View(), m.Message)
}

// ---------------------------------------------------------------------------
// Helper: count display
// ---------------------------------------------------------------------------

// CountDisplay returns a colored count display like "12 to update, 3 to move".
func CountDisplay(updates, moves, clones int) string {
	var parts []string
	if updates > 0 {
		parts = append(parts, fmt.Sprintf("%s %s",
			SuccessStyle.Render(fmt.Sprintf("%d", updates)),
			LabelStyle.Render("to update"),
		))
	}
	if moves > 0 {
		parts = append(parts, fmt.Sprintf("%s %s",
			WarnStyle.Render(fmt.Sprintf("%d", moves)),
			LabelStyle.Render("to move"),
		))
	}
	if clones > 0 {
		parts = append(parts, fmt.Sprintf("%s %s",
			TitleStyle.Render(fmt.Sprintf("%d", clones)),
			LabelStyle.Render("to clone"),
		))
	}
	return strings.Join(parts, ", ")
}
