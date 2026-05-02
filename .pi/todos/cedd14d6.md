{
  "id": "cedd14d6",
  "title": "Port execute package (action execution with concurrency)",
  "tags": [],
  "status": "done",
  "created_at": "2026-04-30T23:22:15.883Z",
  "parent_id": "551a7879"
}

## What

Port `gitjuggling/src/execute.rs` to `internal/execute/execute.go`.

## Source

Rust: `gitjuggling/src/execute.rs` — parallel action execution with rayon, git stash/pull/clone/move, progress bar, summary printing.

## Target

Go: `internal/execute/execute.go`

## Details

### Structs

```go
type ActionResult struct {
    Description string
    Success     bool
    Message     string
}

type ExecuteOptions struct {
    DryRun       bool
    Interactive  bool
    Concurrency  int
}
```

### Functions

- `ExecuteActions(actions []syncplan.Action, opts ExecuteOptions) []ActionResult` — run actions with bounded concurrency using `golang.org/x/sync/semaphore`. Each action in a goroutine, results collected via channel. **No progress bar output here** — the TUI layer handles that. This function should accept a callback or return results via channel so the TUI can display progress.

  Design: use a channel-based approach:
  ```go
  func ExecuteActions(ctx context.Context, actions []syncplan.Action, opts ExecuteOptions) <-chan ActionResult
  ```
  The caller reads from the channel to get results as they complete. Internally uses semaphore for concurrency control.

### Action execution (private functions)

- `executeUpdate(repo, localPath string) ActionResult` — run `git stash -u` then `git pull --rebase` in `localPath`
- `executeMove(repo, currentPath, expectedPath string, interactive bool) ActionResult` — optionally prompt, create parent dir with `os.MkdirAll`, `os.Rename`
- `executeClone(repo, cloneURL, expectedPath string) ActionResult` — create parent dir, run `git clone <url> <path>`
- `dryRunAction(action) ActionResult` — return description of what would happen

### Git subprocess helpers

All git commands use `exec.Command("git", args...)` with `cmd.Dir` set to the repo path. Capture stdout/stderr.

### Interactive confirmation

The `interactive` parameter on `executeMove` should be a callback function `func(prompt string) bool` so the TUI layer can provide the Bubbletea-based confirmation dialog. For non-TUI usage, a simple stdin-based fallback.

### No tests to port

Rust code has no unit tests for execute. Can be tested via integration tests later.
