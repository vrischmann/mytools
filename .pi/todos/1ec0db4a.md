{
  "id": "1ec0db4a",
  "title": "Create TUI styles and shared Bubbletea components",
  "tags": [],
  "status": "done",
  "created_at": "2026-04-30T23:23:30.476Z",
  "parent_id": "551a7879"
}

## What

Create `internal/tui/styles.go` and `internal/tui/components.go` with shared Bubbletea styles and reusable components.

## Target

Go: `internal/tui/styles.go`, `internal/tui/components.go`

## Details

### `styles.go` — Lipgloss style definitions

Match the current Rust color scheme:

```go
var (
    // Labels/headers
    LabelStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("12"))  // blue
    // Success/checkmarks
    SuccessStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("10")) // green
    // Failures
    ErrorStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("9"))   // red
    // Titles
    TitleStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("14"))  // cyan
    // Moves/warnings
    WarnStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("11"))   // yellow
    // Dimmed/secondary
    DimStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("8"))     // gray

    // Section header: "=== Title ==="
    SectionStyle = lipgloss.NewStyle().
        Foreground(lipgloss.Color("15")).  // bright white for "==="
        Bold(true)

    // Custom colors matching Rust TrueColor definitions
    StdoutColor = lipgloss.Color("#b0b0b0")  // {r:176, g:176, b:176}
    StderrColor = lipgloss.Color("#db9a9a")  // {r:219, g:154, b:154}
)
```

Helper functions:
- `SectionHeader(title string) string` — format `=== Title ===` with appropriate styles
- `Checkmark() string` — green `✓`
- `CrossMark() string` — red `✗`
- `Arrow() string` — blue `→`

### `components.go` — Shared Bubbletea components

Reusable models:

- **ConfirmModel** — Yes/No confirmation dialog (replaces `dialoguer::Confirm`). Keys: `y`/`n` or Enter/Esc. Returns the user's choice via a `tea.Msg`.
- **ProgressModel** — Wraps `bubbles.Progress` with a status message (repo name). Used by both `sync` and `exec` commands.
- **SpinnerModel** — Wraps `bubbles.Spinner` with a message. Used for loading phases ("Fetching GitHub repos...").

Each component should be a self-contained Bubbletea model that can be embedded in larger models.
