{
  "id": "79eedadb",
  "title": "Port discover package (local repo discovery + URL normalization)",
  "tags": [],
  "status": "done",
  "created_at": "2026-04-30T23:21:24.868Z",
  "parent_id": "551a7879"
}

## What

Port `gitjuggling/src/discover.rs` to `internal/discover/discover.go`.

## Source

Rust: `gitjuggling/src/discover.rs` — `LocalRepo`, `LocalRepos` structs, filesystem walk, `git remote get-url` shelling out, URL normalization.

## Target

Go: `internal/discover/discover.go` + `internal/discover/discover_test.go`

## Details

### Structs

```go
type LocalRepo struct {
    Path        string
    RemoteURLs  map[string]string  // remote name → URL
}

type LocalRepos struct {
    Repos  []*LocalRepo
    ByURL  map[string]string       // normalized URL → local path
    ByName map[string][]string     // dir name → list of paths
}
```

### Functions

- `Discover(root string) (*LocalRepos, error)` — walk `root` using `filepath.WalkDir`, find `.git` dirs, shell out to `git remote get-url origin`, build lookup maps
- `(lr *LocalRepos) FindByURL(url string) (string, bool)` — try normalized URL and raw URL against `ByURL`
- `normalizeURL(url string) string` — convert SSH/HTTPS forms to canonical `host/owner/repo` format
- `getRemoteURL(repoPath, remote string) (string, error)` — run `git remote get-url <remote>` in the given directory

### URL normalization rules (same as Rust)

| Input | Output |
|---|---|
| `git@github.com:owner/repo.git` | `github.com/owner/repo` |
| `https://github.com/owner/repo.git` | `github.com/owner/repo` |
| `https://github.com/owner/repo/` | `github.com/owner/repo` |
| `github.com/owner/repo` | `github.com/owner/repo` (idempotent) |

### Tests to port

- `test_normalize_ssh_url`
- `test_normalize_https_url`
- `test_normalize_forgejo_url`
- `test_normalize_trailing_slash`
- `test_normalize_idempotent`
