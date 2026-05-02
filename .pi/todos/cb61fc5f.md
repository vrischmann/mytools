{
  "id": "cb61fc5f",
  "title": "Build exec command Bubbletea TUI",
  "tags": [],
  "status": "done",
  "created_at": "2026-04-30T23:23:49.798Z",
  "parent_id": "551a7879"
}

## What

Replace the simple `fmt.Println` output in `cmd/exec.go` with a Bubbletea TUI model.

## Target

Go: `internal/tui/exec.go` (Bubbletea model for exec command), updates to `cmd/exec.go`

## Details

### Model structure

```go
type execModel struct {
    phase       execPhase  // discovering | running | done
    total       int
    completed   int
    currentRepo string
    progress    progress.Model
    results     []execItem
    verbose     bool
    quitting    bool
}

type execPhase int
const (
    phaseDiscovering execPhase = iota
    phaseRunning
    phaseDone
)

type execItem struct {
    path    string
    success bool
    stdout  string
    stderr  string
    err     error
}
```

### View layout

**Discovering phase:**
```
  Scanning repos...
```
With a spinner.

**Running phase:**
```
  [████████████░░░░░░] 12/20  myrepo
```
Progress bar with current repo name.

**Done phase (verbose):**
```
  === Output ===

  /path/to/repo1
  <stdout in gray>
  <stderr in muted red>

  /path/to/repo2
  ...

  === Summary ===
  Succeeded: 18
  Failed:    2
  Time:      3.45s
```

**Done phase (non-verbose):**
Only shows failed items and summary.

### Messages

Use a channel-based pattern: the exec goroutines send results on a channel, and the Bubbletea model polls it via `tea.Tick` (every 100ms).

```go
type resultMsg execItem
type doneMsg struct{}
```

### Key bindings

- `q` or `ctrl+c` — quit

### Integration with cmd/exec.go

The `cmd/exec.go` RunE function should:
1. Create the Bubbletea model
2. Start goroutines for discovery and execution (sending results via channel)
3. Run `tea.Program.Run()`
4. After the program exits, check for failures and set exit code
