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
    forgejo_url: https://git.example.com
    forgejo_user: vincent
    forgejo_token: "op://vault/item/field"
    rules:
      base: /home/user/dev/repos
      forks: /home/user/dev/forks
      archived: /home/user/dev/archived

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
