# gitjuggling

A tool to sync local git repositories with upstream GitHub/Forgejo remotes and run git commands across multiple repositories.

## Commands

### `sync` — Sync repos with upstream

```
$ gitjuggling sync [workspace] [flags]
```

Discovers remote repos from GitHub/Forgejo, compares with local repos, and executes update/move/clone actions as needed.

Flags:
- `--config` — path to config file (default: `~/.config/gitjuggling/config.yaml`)
- `--dry-run` — show what would be done without making changes
- `--interactive` — prompt before destructive actions (default: true)
- `--prune` — remove local repos with no upstream match
- `--skip-pull` — skip `git pull` for repos already in the expected location
- `-c, --concurrency` — concurrency limit (default: 2)

### `exec` — Run a git command in all repos

```
$ gitjuggling exec -- <git args...>
```

Flags:
- `-d, --depth` — search depth for repository discovery (default: 3)
- `-c, --concurrency` — concurrency limit (default: 2)
- `-v, --verbose` — show output from all repos, not just failures

## Configuration

Routing is decided per repo, in this priority order:

1. **Pattern rules** (`rules.patterns`) — the first regex matching the repo
   name wins. The `to` template is the repo's **full final path** and must
   reference at least one capture (`$1`, `${name}`, or `$0` for the whole
   match), so distinct repos never collapse onto one directory. Example:
   `^workspace-(.+)$` with `to: workspaces/$1` sends `workspace-foo` to
   `workspaces/foo`.
2. **Category rules** — `forks` for forks, then `archived` for archived
   repos, then `base` for everything else.

If two repos would land on the same path, the owner is prefixed to the final
   directory component to disambiguate (e.g. `owner-foo`).

Config file location: `<UserConfigDir>/gitjuggling/config.yaml`

On Linux: `~/.config/gitjuggling/config.yaml`
On macOS: `~/Library/Application Support/gitjuggling/config.yaml`

Example:

```yaml
default_workspace: personal

workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    github_token: "ghp_..."  # Optional: direct GitHub token, falls back to `gh auth token`
    forgejo_url: https://git.example.com
    forgejo_user: vincent
    forgejo_token: "forgejo_token_here" # Or an `op://vault/item/field` reference
    rules:
      base: /home/user/dev/repos
      forks: /home/user/dev/forks
      archived: /home/user/dev/archived
      patterns:
        # Routes workspace-* repos into workspaces/<suffix>; the `to`
        # template is the repo's full final path and may reference
        # capture groups ($1, ${name}, or $0 for the whole match).
        - pattern: '^workspace-(.+)$'
          to: /home/user/dev/workspaces/$1

  work:
    root: /home/user/work
    github_owners: [MyOrg]
    rules:
      base: /home/user/work/repos
```

## Build

```
go build -o gitjuggling .
```

Or use the justfile:

```
just build    # build binary
just test     # run tests
just check    # run go vet
just install  # install to $GOPATH/bin
```
