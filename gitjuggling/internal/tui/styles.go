package tui

import (
	"fmt"

	"github.com/charmbracelet/lipgloss"
)

var (
	// LabelStyle is used for labels/headers.
	LabelStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("12")) // blue

	// SuccessStyle is used for success messages and checkmarks.
	SuccessStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("10")) // green

	// ErrorStyle is used for error messages and cross marks.
	ErrorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("9")) // red

	// TitleStyle is used for section titles.
	TitleStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("14")) // cyan

	// WarnStyle is used for moves and warnings.
	WarnStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("11")) // yellow

	// DimStyle is used for dimmed/secondary info.
	DimStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("8")) // gray

	// BoldStyle is used for emphasis.
	BoldStyle = lipgloss.NewStyle().Bold(true)

	// SectionStyle is used for the === delimiters in section headers.
	SectionStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("15")).Bold(true) // bright white

	// StdoutStyle is used for git command stdout.
	StdoutStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#b0b0b0"))

	// StderrStyle is used for git command stderr.
	StderrStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("#db9a9a"))
)

// SectionHeader formats a section header like "=== Title ===".
func SectionHeader(title string) string {
	return fmt.Sprintf("%s%s%s",
		SectionStyle.Render("=== "),
		TitleStyle.Bold(true).Render(title),
		SectionStyle.Render(" ==="),
	)
}

// Checkmark returns a green ✓.
func Checkmark() string {
	return SuccessStyle.Render("✓")
}

// CrossMark returns a red ✗.
func CrossMark() string {
	return ErrorStyle.Render("✗")
}

// Arrow returns a blue →.
func Arrow() string {
	return LabelStyle.Render("→")
}

// Bullet returns a dimmed ●.
func Bullet() string {
	return DimStyle.Render("●")
}

// PendingBullet returns a dimmed ○.
func PendingBullet() string {
	return DimStyle.Render("○")
}
