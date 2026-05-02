{
  "id": "bf1a0be5",
  "title": "Build sync command Bubbletea TUI (multi-phase)",
  "tags": [],
  "status": "done",
  "created_at": "2026-04-30T23:24:20.823Z",
  "parent_id": "551a7879"
}

## What

Replace the simple `fmt.Println` output in `cmd/sync.go` with a multi-phase Bubbletea TUI model.

## Target

Go: `internal/tui/sync.go` (Bubbletea model for sync command), updates to `cmd/sync.go`

## Details

### Model structure

```go
type syncModel struct {
    phase       syncPhase  // loading | plan | executing | summary
    workspace   string

    // Loading phase
    spinner     spinner.Model
    loadSteps   []loadStep  // track completion of each loading step

    // Plan phase
    actions     []syncplan.Action
    updates     int
    moves       int
    clones      int
    cursor      int         // for plan review
    confirmed   bool

    // Execution phase
    progress    progress.Model
    completed   int
    currentDesc string
    results     []execute.ActionResult

    // Summary phase
    succeeded   []execute.ActionResult
    failed      []execute.ActionResult

    // Options
    dryRun      bool
    interactive bool
    doPrune     bool

    // Prune sub-phase
    orphans     []*prune.OrphanRepo
    prunePhase  prunePhase  // listing | confirming | done
    pruneCursor int
    pruneResults []*prune.PruneResult

    quitting    bool
}
```

### Phases

**Phase 1 — Loading:**
```
  ● Loading config...
  ● Fetching GitHub repos... ⟳
  ○ Fetching Forgejo repos...
  ○ Scanning local repos...
```
Completed steps show `✓`, current step shows a spinner, pending steps show `○`.

**Phase 2 — Plan review:**
```
  === Plan ===
  12 to update, 3 to move, 5 to clone

    Update:
      ✓ owner/repo1
      ✓ owner/repo2

    Move:
      → owner/repo3  /old/path → /new/path

    Clone:
      + owner/repo4  → /repos/owner/repo4

  [Enter] Execute  [q] Quit
```

**Phase 3 — Execution:**
```
  [████████████░░░░░░] 15/20  updating owner/repo2
```

**Phase 4 — Summary:**
```
  === Sync Summary ===

  Succeeded:
    ✓ owner/repo1
    ✓ owner/repo2 (cloned)

  Failed:
    ✗ owner/repo3
      git pull failed: ...

  Succeeded: 18 | Failed: 2
```

**Phase 5 — Prune (if --prune):**
```
  Found 3 orphan repos

    ● repo-old (~/dev/repos/owner/repo-old)
    ● repo-deprecated (~/dev/repos/owner/repo-deprecated)

  [y] Remove all  [n] Skip  [i] Interactive
```

In interactive mode, cycle through orphans:
```
  Remove orphan repo-old? (~/dev/repos/owner/repo-old)
  [y] Yes  [n] No  [a] Yes to all  [q] Quit
```

### Messages

```go
type configLoadedMsg *config.Config
type githubReposMsg []*remote.RemoteRepo
type forgejoReposMsg []*remote.RemoteRepo
type localReposMsg *discover.LocalRepos
type planBuiltMsg []syncplan.Action
type actionResultMsg execute.ActionResult
type pruneResultMsg prune.PruneResult
type errMsg error
```

### Key bindings

- Plan phase: `Enter` = confirm, `q` = quit, `↑/↓` = scroll plan list
- Prune phase: `y/n/a/q` for interactive confirmation
- Summary phase: `q` = quit

### Concurrency integration

Loading phase runs API calls and discovery concurrently via goroutines, sending results as `tea.Msg`. Execution phase uses the `execute.ExecuteActions` channel-based API, polling via `tea.Tick`.

### Integration with cmd/sync.go

The `cmd/sync.go` RunE function creates the Bubbletea model, wires up the initial options, and runs `tea.Program.Run()`. After exit, check for errors to set exit code.
