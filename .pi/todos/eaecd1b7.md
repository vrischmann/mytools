{
  "id": "eaecd1b7",
  "title": "Port remote package (GitHub/Forgejo API clients + dedup)",
  "tags": [],
  "status": "done",
  "created_at": "2026-04-30T23:21:40.372Z",
  "parent_id": "551a7879"
}

## What

Port `gitjuggling/src/remote.rs` to `internal/remote/remote.go`.

## Source

Rust: `gitjuggling/src/remote.rs` — `RemoteRepo`, GitHub/Forgejo API clients with pagination, token retrieval, deduplication.

## Target

Go: `internal/remote/remote.go`

## Details

### Structs

```go
type RemoteSource int

const (
    SourceGitHub RemoteSource = iota
    SourceForgejo
)

type RemoteRepo struct {
    Name      string
    Owner     string
    IsFork    bool
    IsArchived bool
    IsMirror  bool
    CloneURL  string
    Source    RemoteSource
}
```

### Functions

- `(r *RemoteRepo) DedupKey() [2]string` — lowercase `(owner, name)` tuple for dedup

#### GitHub client

- `githubToken() (string, error)` — run `gh auth token`, return trimmed stdout
- `FetchGitHubRepos(owners []string) ([]*RemoteRepo, error)` — for each owner, paginate through `/orgs/{owner}/repos` (fall back to `/users/{owner}/repos` on 404), 100 per page
- Internal: `fetchGitHubOwnerRepos(token, owner string) ([]*RemoteRepo, error)` — pagination loop, parse JSON response into `RemoteRepo`

#### Forgejo client

- `forgejoToken(cmd string) (string, error)` — run `sh -c <cmd>`, return trimmed stdout
- `FetchForgejoRepos(baseURL, user, tokenCmd string) ([]*RemoteRepo, error)` — paginate through `/api/v1/users/{user}/repos`, 50 per page, **skip mirrors**

#### Deduplication

- `DedupRepos(github, forgejo []*RemoteRepo) ([]*RemoteRepo, []*RemoteRepo)` — remove Forgejo repos whose `DedupKey()` matches any GitHub repo. Returns modified slices.

### HTTP client

Use stdlib `net/http` + `encoding/json`. Set `User-Agent: gitjuggling` header on all requests. Use `http.Client` with default timeouts.

### No tests to port

The Rust code has no unit tests for remote — integration-level only. No tests needed here.
