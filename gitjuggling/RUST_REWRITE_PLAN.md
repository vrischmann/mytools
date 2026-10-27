# gitjuggling → Rust rewrite plan

Goal: replace the Go implementation with a Rust crate in the Cargo workspace, at
behavior parity (same config file, same plan file, same CLI surface, same TUI
phases), then delete the Go tree.

---

## 1. Current-state inventory

`gitjuggling/` — 31 `.go` files, ~5,975 LOC (≈3,400 code + ≈2,500 tests), 62 test
functions. Not a workspace member; built separately (`go build ./...`).

| Go package | LOC | Responsibility |
|---|---|---|
| `main.go` + `cmd/` (root, sync, exec, init) | 366 | CLI (cobra): `sync [workspace]`, `exec -- <git args…>`, `init` |
| `internal/config` | 245 | YAML config, workspace lookup, ignore globs, pattern-rule validation |
| `internal/syncplan` | 250 | Classification (fork/archived/base/pattern), clash disambiguation, `BuildPlan`, worktree-prune proposals |
| `internal/discover` | 250 | FS walk, `.git` dir vs `.git` file, worktree vs submodule, URL index |
| `internal/execute` | 558 | Every mutating git call + bounded-concurrency runner + GitHub askpass helper |
| `internal/git` | 88 | Read-only git helpers (branch, dirty, default branch, merged, ls-remote) |
| `internal/remote` | 302 | GitHub + Forgejo API clients, pagination, dedup, `op://` + `gh` token resolution, https→ssh |
| `internal/syncstate` | 99 | Durable per-workspace plan JSON (atomic write, 0600) |
| `internal/prune` | 121 | Orphan detection + removal |
| `internal/gitmodules` | 175 | Hand-written `.gitmodules` INI-ish parser |
| `internal/tui` | 1,939 | Bubbletea models: `sync.go` (1,417), `exec.go` (262), `components.go`, `styles.go` |
| `internal/gittest` | 54 | Test-only real-git fixture builder |

### CLI surface (must not change)

```
gitjuggling sync [workspace] [--config P] [--dry-run] [--interactive=true]
                             [--prune] [--skip-pull] [-c 2]
gitjuggling exec [-d 3] [-c 2] [-v] -- <git args...>
gitjuggling init [-c P]
```

Exit codes are **not** "1 if anything failed" — the real rule is narrower, and a
Rust `fn main() -> anyhow::Result<()>` gets it wrong by default. Matrix (see §4.11):

| outcome | exit |
|---|---|
| config/workspace/plan-load error *before* the TUI starts | 1 |
| any entry in `m.failed` (incl. a tracking fix the user **declined**) | 1 |
| a load error raised *after* the TUI started (`loadErr`) | 0 |
| every **prune** failure (`pruneResults`) | 0 |
| success / dry-run / user quit | 0 |

Note `-c` means *concurrency* on `sync`/`exec` but *config path* on `init`.

### sync phase machine (12 phases)

`loading` (GitHub → Forgejo → local scan + dirty check, sequential, spinner) →
`resume` (when `plans/<ws>.json` exists) → `plan` (scrollable viewport) →
`dirty confirm` → `move confirm` → `worktree confirm` → `executing` (bounded
concurrency, live log) → `tracking fix` (sequential prompts) → `prune list` →
`prune confirm` → `prune done` → `summary`. `q`/`ctrl+c` aborts; confirm phases
support `y` / `n` / `a` (all remaining) / `q` (back to plan).

### Durable plan

`<UserConfigDir>/gitjuggling/plans/<workspace>.json`. Written after plan build,
one action removed per success, deleted when empty → resume retries only
unfinished work. **Never written in dry-run mode** (`sync.go:740-751`), which
matters for the §6 harness. Resume takes the saved actions *and skips discovery
entirely* — no local scan, so no dirty map, so **no dirty confirmation** on a
resumed run. User-declined moves and dirty updates are recorded as *successes* and
therefore removed from the plan: resume does not re-offer them.

Serialization shape is a hard contract, not an implementation detail (§7F, and
confirmed from the real file on disk): PascalCase field names, enums as ints
(`Update`=0…`PruneWorktree`=3; `GitHub`=0/`Forgejo`=1), no omitted fields,
`{"actions": […]}` with 2-space indent.

A stale `plans/.json` (empty workspace name) sits in the config dir; it is
unreachable in current Go (`GetWorkspace` rejects an empty name before `Load`
runs), so no code needs to handle it — delete it by hand at cutover.

### Config file (your live one parses, must keep parsing)

`default_workspace`, `workspace.<name>.{root, github_owners, github_token,
forgejo_url, forgejo_user, forgejo_token, rules.{base, forks, archived,
patterns[{pattern, to}]}, ignore[]}`.

### Credential paths

- GitHub: `github_token` → else `gh auth token`.
- Forgejo: literal, or `op://…` resolved via `op read --account my.1password.eu <ref>`.
- Clones of GitHub repos: temp `GIT_ASKPASS` script (0700) +
  `GITJUGGLING_GITHUB_TOKEN` env + `GIT_TERMINAL_PROMPT=0` + `-c credential.helper=`
  so the token never lands in argv or the remote URL.

---

## 2. Target crate layout

```
gitjuggling/            # replaces the Go dir in place, becomes a workspace member
├── Cargo.toml          # edition 2024, clap/anyhow/ratatui/ureq/serde…
├── README.md           # keep, update Build section
└── src/
    ├── main.rs         # clap entry, exit codes
    ├── cli/{sync.rs, exec.rs, init.rs}
    ├── config.rs       # Config/Workspace/Rules/PatternRule, validate, IsIgnored
    ├── urlnorm.rs      # NormalizeURL, https→ssh
    ├── gitmodules.rs
    ├── discover.rs     # walk + LocalRepo/LocalRepos + indices
    ├── git.rs          # all git access as subprocesses (decision G), env/askpass wrapper
    ├── remote/{github.rs, forgejo.rs, secrets.rs}
    ├── plan.rs         # classify + BuildPlan + Action/ActionType
    ├── state.rs        # durable plan JSON
    ├── exec.rs         # mutating git ops, dry-run, bounded runner
    ├── prune.rs
    └── tui/{app.rs, sync.rs, exec.rs, view.rs, styles.rs, widgets.rs}
```

Keep `internal/gittest`'s equivalent as `src/testing/fixtures.rs` (behind
`#[cfg(test)]`) or `tests/common/mod.rs`.

Two pieces of Go code are **dead — do not port them**: `discover.LocalRepos.byName`
is built and indexed but never read (`discover.go:32,40,58,66`), and
`tui.ConfirmModel`/`NewConfirmModel` is never instantiated (`components.go:24-77`;
the sync TUI implements its own per-phase confirmation instead). Note it here so
nobody wastes time hunting for their callers.

## 3. Dependency mapping

| Concern | Go | Rust choice | Notes |
|---|---|---|---|
| CLI | cobra | `clap` 4 derive | matches the rest of the workspace |
| TUI | bubbletea + bubbles + lipgloss | `ratatui` 0.30 + `crossterm` 0.29 | see decision A |
| YAML | gopkg.in/yaml.v3 | `serde_yaml_ng` 0.10 + `serde` | maintained fork of `serde_yaml` (libyaml-backed); `serde_yaml` itself is deprecated/unmaintained |
| JSON | encoding/json | `serde_json` | plan file (shape contract §7F), API responses |
| HTTP | net/http | `ureq` 3 (blocking) | no async runtime needed; **must set an explicit 30s timeout** — ureq has no default (§4.12) |
| Git | `os/exec` on `git` | `std::process::Command` on `git` | decision G: subprocess only, **no git2**; port the env/askpass wrapper from `internal/git.go` |
| Regex | regexp | `regex` 1 | see §4.1 |
| Glob | filepath.Match | port Go's `filepath.Match` (~100 LOC) | `glob` crate is *not* parity-equivalent — see §4.4 |
| FS walk | filepath.WalkDir | `jwalk` 0.8 (already used in workspace) or `walkdir` | jwalk = parallel dirty checks for free |
| Concurrency | goroutines + semaphore | `std::thread::scope` + `sync::mpsc` + a semaphore | see decision B |
| Paths | path/filepath | `std::path` + `camino` (optional) | |
| Config dir | os.UserConfigDir | `dirs` 6 (already used by hf-cache) | same macOS/Linux resolution |
| Errors | wrapped fmt.Errorf | `anyhow` (see decision C) | |
| Tests | stretchr/testify | `tempfile` + `assert_cmd`/`predicates` (optional) | |

Versions pinned by inspection of the registry: `ratatui` 0.30.2 (MSRV 1.88 —
workspace is on 1.97.1, fine), whose `ratatui-crossterm` backend defaults to
`crossterm` 0.29. Ratatui ships **no spinner and no bubbles-style viewport
keymap** (the plan view uses pgup/pgdn, b/f, u/d, j/k) — both must be written,
which is part of decision A.

No async runtime: nothing here needs one — HTTP is blocking `ureq`, git is
subprocess, the walk is `jwalk`. The workspace currently has zero tokio.

## 4. Behavior-parity checklist — the tricky parts

These are the places where a naive port silently changes behavior. Each needs a
test that pins the Go behavior, not just "reasonable Rust".

### 4.1 Pattern-rule template expansion (`rules.patterns[].to`)

Go uses `regexp.ExpandString`, and `Rules.Validate()` re-implements Go's internal
`extract` to *reject* templates that reference a non-existent group — because a
template that expands every repo to the same string would silently collapse
distinct repos onto one directory.

The Rust `regex` crate cannot enforce that, whichever expansion entry point you
use. **Empirically verified against regex 1.13.1**, both `Regex::replace` and
`Captures::expand` (the `ExpandString` equivalent; `Regex::expand` itself no
longer exists) substitute the **empty string** for unknown references:

```
re = ^workspace-(.+)$        template          result
                             "workspaces/$1"   workspaces/foo
                             "workspaces/${1}" workspaces/foo
                             "workspaces/$name"  workspaces/   ← collapse!
                             "a/$2/b"          a//b            ← collapse!
                             "${nope}"         ""              ← collapse!
                             "p/$$1"           p/$1
                             "x/$0/y"          x/workspace-foo/y
```

So `expand` existing buys nothing: it *cannot fail*, which is the whole
requirement. A reference parser is needed regardless. Write it once and use it in
both places:

- **validation** — parse refs, reject any `$n` with `n > num_groups` or any
  `${name}` that is not a named group, and require ≥1 reference (port
  `validatePatternTemplate` + `extractRef`, `config.go:116-215`);
- **expansion** — either drive it off that same parse, or call `caps.expand`
  *after* validation has proved every ref resolves.

Mirror Go's numeric-parse quirks: `$$` is an escaped dollar; a name with leading
zeros (other than `"0"`) or a value ≥ 1e8 is treated as a named reference, not a
number.

Port `TestRulesValidate` (config_test.go:180) and the 6 `ClassifyPattern*` tests
verbatim, and add a table test that compares Rust expansion against the Go binary's
output for the pathological templates above (`$0a`, `${1}`, `$$1`, `$name`, `$9`).

### 4.2 Regex dialect

Both Go RE2 and Rust `regex` lack look-around and backreferences, so most patterns
port as-is; a few character-class and `\p{…}` idioms differ. Add a canary test that
compiles the pattern from the user's real config (`^workspace-(.+)$` today) and
asserts the expanded path — but use a **redacted copy of the real config committed
as a fixture**, not `~/Library/.../config.yaml`: CI cannot read the live file, and
a test that silently skips when a file is missing proves nothing. Mask nothing
beyond what you must; an `op://…` reference is a pointer, not a secret.

### 4.3 Worktrees must not shadow their main clone

`discover` keeps worktrees **out of** `byURL`/`byPath` (and `byName`, which is
dead — §2) (fix e96bff2) and
`prune` skips them (removing one with a recursive delete corrupts the parent's
worktree metadata). Also: a `.git` **file** is a worktree only when its `gitdir:`
resolves to `<main>/.git/worktrees/<name>`; submodule checkouts (`<parent>/.git/modules/…`)
must be skipped. Repos with no `origin` are skipped entirely.

### 4.4 Ignore globs

Go's `filepath.Match`: `*` does **not** cross `/`, matching is case-insensitive
(both sides lowercased), and a malformed pattern is treated as *no match*, not an
error. The `glob` crate is **not** parity-equivalent (no backslash escapes,
different `[!…]` handling), and Go's matcher additionally supports classes
(`[a-z]`), negation (`[!…]`) and escapes — more than a ~40-LOC improvised matcher
will get right.

→ Port Go's `filepath.Match` directly (~100 LOC, BSD license + attribution
comment) and keep the `TestIsIgnored` table verbatim (`config_test.go:128-164`).

### 4.5 Plan determinism

Go iterates a `map[string]*Workspace` (random order) but `BuildPlan` walks the
remote-repo slice in API order, so action order is stable today. Use `BTreeMap`
(or `indexmap`) for `workspace:` so error messages and any future map iteration
stay deterministic, and preserve remote-repo order for action order — the TUI
list and the plan JSON depend on it.

### 4.6 YAML dialect

`yaml.v3` accepts YAML 1.1-ish input your file doesn't currently use but might
(unquoted `yes`, tabs-in-comments, etc.). `serde_yaml_ng` continues `serde_yaml`
(libyaml-backed) and carries serde's data model, which differs at the margins.
Add a fixture test that parses a **committed redacted copy** of the real config and
asserts every field, including `op://` token strings and 4-space-indented maps.

### 4.7 Dirty check + walk cost

Go runs `git status --porcelain` serially for every discovered repo inside the
scan (`tui/sync.go:251-261`). Parallelising it with `jwalk`/rayon is a genuine
improvement, but only with three conditions held:

- **Coverage identical**: `local.Iter()` **includes worktrees**, so the dirty map
  must cover them too — a worktree's dirt belongs to its own checkout.
- **Definition identical**: `IsDirty` ≡ non-empty `git status --porcelain`,
  **including untracked**. `git2::Repository::statuses` needs the untracked
  option enabled and still diverges from the CLI on submodule/ignore-file edge
  cases. Safest: keep shelling out to `git status --porcelain`.
- **Bounded parallelism**: rayon's default = all cores, which would spawn one
  `git status` per core over large trees. Use the same pool bound as `-c`.

Related: `discover` itself shells out `git remote get-url origin` per candidate and
**skips the repo when that fails** (`discover.go:147-152`). Reading remotes with
`git2` instead changes *which repos exist*, which is a plan-visible change — don't.

### 4.8 exec's discovery quirks

`cmd/exec.go` walks `.` with a depth limit expressed as a *separator count on the
walk path*, and keeps the **last** `.gitmodules` found anywhere in the walk, then
skips a repo when the basename of its parent dir equals a submodule `path`. Three
further quirks ride along:

- it accepts only `.git` **directories**, so `exec` never visits linked worktrees
  even though `sync` prunes them (`cmd/exec.go:143-146`);
- the submodule filter compares a *basename* against a submodule `path`, so nested
  submodule paths (`vendor/foo`) never match;
- `CombinedOutput` merges stdout+stderr into whichever field the exit status picks,
  so an "error" line may contain stdout and vice versa.

Decision E — DECIDED (2026-10-26): fix the `.gitmodules` lookup (nearest file,
per repo) and **also** make `exec` visit linked worktrees (accept `.git` files,
not just directories); both are recorded in §8. `CombinedOutput` semantics and the
basename-vs-path match quirk are kept. The §6.3 exec comparison must therefore
run on a tree without linked worktrees (or filter worktree paths), since Rust
visits strictly more repos than Go.

### 4.9 Secrets + subprocess hygiene

The askpass helper (temp file 0700, `case` on "Username"/"Password", cleanup on drop)
has to survive panics in the worker thread — model cleanup with a guard type.
Never put the token in argv or the clone URL.

### 4.10 Failure semantics to preserve verbatim

- `IsNoTrackingError` / `IsStaleUpstreamError` are **substring** matches on git's
  combined output — keep the exact strings, they drive the tracking-fix phase.
  (Note: `"There is no tracking information"` contains `"no tracking information"`,
  so the second check is redundant; keep one and say why.)
- `ResolveStaleUpstream` refuses on the default branch; deletes the stale branch
  only when merged.
- `RemoveWorktreeAndBranch` deletes the branch only when it was merged **at plan
  time**; unmerged → branch kept, still reported as success.
- Move falls back to in-place update when the expected path is already occupied by
  another local repo.
- Clash → owner prefix on the **final component only** (`dir/owner-name`).
- **Update = `git stash -u` then `git pull --rebase`, and the stash is never
  popped** (`execute.go:404-425`; the TUI copy admits it: "recoverable via `git
  stash pop`"). Do not "improve" this into a pop — it changes what the user finds
  in their working tree. Pin it with a test asserting the stash entry still exists
  after an update.
- **`--interactive=false` (or `--dry-run`) skips *every* confirmation, including
  the dirty-repo prompt** (`enterFromPlan`, `sync.go:286-291`). A non-interactive
  run therefore stashes uncommitted work with no prompt at all. Parity requires
  keeping that; a port that prompts anyway changes data semantics.
- **Declined ≠ failed for the plan file**: user-declined moves and dirty updates are
  recorded as successes, so `completeAction` drops them from the durable plan and
  resume will not re-offer them. The result→action match is by **path**, plus a
  literal `"skipped (user declined)"` message comparison for declined moves
  (`sync.go:386-398` ↔ `execute.go:443`) — string-coupled control flow. Replace it
  with an action id or `(path, kind)` key while keeping observable behavior, and add
  tests "declined move is not retried on resume" and "resumed run has no dirty
  confirm".

### 4.11 Exit-code contract (see §1 matrix)

`HasFailures` inspects only `m.failed`. Consequently prune failures exit **0**
(`pruneResults` are never merged into `m.failed`), a **declined tracking fix**
exits **1** (it is appended to `m.failed`), and a load error raised *after* the TUI
started exits **0** while the same error *before* the TUI exits 1. A Rust
`fn main() -> anyhow::Result<()>` inverts two of these three. Encode the matrix as
a test in M5/M6.

### 4.12 HTTP contract (GitHub + Forgejo)

Not "call the API" — the details are load-bearing:

| | GitHub | Forgejo |
|---|---|---|
| endpoint | `api.github.com/user/repos` (authenticated-user listing, so private repos appear) | `<base>/api/v1/users/<user>/repos` |
| auth header | `Authorization: Bearer <token>` | `Authorization: token <token>` |
| `User-Agent` | `gitjuggling` — **GitHub 403s requests with no UA** | not set |
| page size | `per_page=100` + `affiliation=owner,collaborator,organization_member` | `limit=50` |
| pagination stop | **empty page**, never `Link` headers | same |
| timeout | 30s per request (client-level, covers the whole exchange) | same |
| filters | owner filter, case-insensitive | mirrors dropped, `clone_url` rewritten to SSH |

`ureq` 3 has **no default timeout**: an unconfigured agent hangs where Go would
have errored at 30s. Set an explicit 30s per-call timeout. Mirror the
empty-page termination exactly — a `Link`-header implementation would fetch more or
fewer pages than the Go original.

### 4.13 Prune with no known remotes

If `github_owners` is empty and Forgejo is unconfigured, both fetches return empty
lists, `FindOrphans` sees zero known URLs, and **every** non-ignored, non-worktree
local repo is offered for deletion — including a workspace full of repos that were
simply never configured. Only the interactive prompt stands between that and data
loss. Decision I — DECIDED (2026-10-26): the port **does not** reproduce this; it
guards (refuse + exit 1 when the known-remote set is empty and orphan candidates
exist, informational under `--dry-run`, no new flag) — see §7 I and §8 item 8.

---

## 5. Phased implementation

Each phase ends green on `cargo fmt --check`, `cargo clippy -D warnings`,
`cargo test -p gitjuggling`. The Go binary stays buildable throughout as the
oracle.

| Phase | Deliverable | Size | Validation |
|---|---|---|---|
| **M0** | Crate skeleton as workspace member; clap stubs for `sync`/`exec`/`init`; deps wired (pin `ratatui` 0.30.2 + `crossterm` 0.29 empirically); CI picks it up. Include a cobra-parity note: bool flags accept `--interactive` / `=true` / `=false`; `sync` takes ≤1 positional; `exec` needs ≥1 arg and passes everything after `--` through verbatim; `help` exists, `completion` is disabled. cobra prints usage on runtime errors, clap does not → stderr differs, accepted (§8). | S | `just check-all`, `cargo run -p gitjuggling -- --help` |
| **M1** | Pure logic: `config` (+validate), `urlnorm`, `gitmodules`, ignore matcher (ported `filepath.Match`), template ref parser + expander, **and the `RemoteRepo` data struct** (needed by M2's `find_orphans`) | M | port config_test, gitmodules_test, remote_test (https→ssh), forgejo_test (token) ≈ 15 tests |
| **M2** | `discover` + `git` read ops + real-git fixture harness + `prune::find_orphans` | M | port discover_test, prune_test, execute_test(IsDirty/RemoveWorktree…) |
| **M3** | `remote` clients (GitHub, Forgejo, `op://`, `gh`) behind an injectable transport, dedup, `plan::build_plan` incl. worktree prune | M | port syncplan_test — the big one (509 LOC / 26 tests); HTTP behind a trait with canned JSON fixtures |
| **M4** | `state` (JSON: new idiomatic format + legacy-Go fallback parse per §7F, atomic, 0600) + `exec` (all git mutations, askpass, bounded runner, dry-run) | M | port execute_test, syncstate_test + a **legacy-load test**: committed Go-written plan JSON parses and migrates in memory + a temp-repo end-to-end: clone/move/update/prune against a local bare "origin" |
| **M5a** | Terminal setup + event loop (decision H) + the sync state machine as a **pure** struct with `handle_key`/`handle_msg`, then loading, resume, plan, summary | M | the 8 `tui/sync_test.go` ports run **headless** against the pure state machine, exactly as they do in Go |
| **M5b** | Confirm phases (dirty / move / worktree) + executing, incl. cancel semantics: `q` stops *scheduling*, in-flight git subprocesses run to completion (no kill-on-drop) | M | port the dirty-confirm keymap test; assert declined-move/declined-dirty removal from the durable plan |
| **M5c** | Tracking-fix phase, prune list/confirm/done, `exec` progress view, and the §4.11 exit-code matrix wired into `main` | M | exit-code matrix test; manual run against a scratch workspace |
| **M6** | Parity verification, docs, cutover | S | §6 harness; update `README.md`, `AGENTS.md`, `justfile` (`go-install-all` → `cargo install --path gitjuggling`), drop the `go` CI job, `git rm` the Go tree + root `go.mod`/`go.sum`, delete the stale `plans/.json` |

The TUI is the bulk of the risk — `tui/sync.go` alone is 1,417 LOC — which is why
M5 is split three ways. The one architectural prerequisite is the event loop
(decision H): crossterm key events, worker-thread `ActionResult`s, spinner ticks
and resize all have to merge into one loop, and the state machine must stay a pure
data structure so it remains testable without a pty. Everything before M5 is
unit-testable without a terminal.

## 6. Parity harness (M6)

As written in the first draft this harness could not run: `--no-interactive` is not
a flag cobra accepts (it is `--interactive=false`), and the plan-JSON diff targeted
a file that dry-run never writes. Corrected, with determinism handled:

**Environment.** Run both binaries under a fixed-size pty (Bubbletea renders an
empty plan view at 0×0 when stdout is a pipe, and lipgloss drops color), with
`NO_COLOR=1`/`TERM=dumb` on both sides, and canonicalize output before diffing:
strip ANSI, drop timing lines, sort the `exec` result blocks (they arrive in
*completion* order, not repo order).

**API drift is the hard limit.** Both binaries fetch repo lists from the **live**
GitHub/Forgejo APIs, and the Go binary has no injection seam — so cross-binary text
diffs of a `sync` plan are only meaningful when the API truth is frozen. Either add
a tiny fixture seam to the Go binary *before* cutover, or scope cross-binary diffs
to what is API-free. Plan:

1. **`exec` parity (fully cross-binary, no API):** same tree, `exec -- status
   --porcelain` under the pty → same repos discovered, same success/failure split,
   same exit code.
2. **`sync` plan parity (dry-run, non-interactive):** `sync --dry-run
   --interactive=false` against a config whose `root:`/`rules.*` point into a temp
   dir seeded with fake origins → diff canonicalized plan text. Diff the persisted
   `plans/<ws>.json` only in the variant below, never after a dry run.
3. **`sync` plan-file parity (non-dry, `--interactive=false`) against local "fake
   origins"**, with the remote listings frozen via the Go fixture seam (or by
   running each binary with the *same* canned transport) → then diff
   `plans/<ws>.json` byte-for-byte between the two binaries.
4. Fixtures cover: clone, update, move, occupied-move, clash, pattern route, ignore
   glob, fork/archived, stale worktree (merged + unmerged), orphans with `--prune`,
   dirty repos, and a resume from a Go-written plan file.
5. Keep the harness in `gitjuggling/tests/parity/` so it survives the cutover.

## 7. Decisions (all nine settled 2026-10-26)

- **A. TUI rendering style — DECIDED (2026-10-26): idiomatic ratatui widgets.**
  `ratatui` 0.30.2 (latest) + `crossterm` 0.29 (ratatui's own default backend version;
  keep the direct dep on the same major). Render each phase with ratatui widgets
  (`Paragraph`/`List`/`Gauge`), NOT Bubbletea's render-to-string style: spacing and
  color drift vs the Go TUI is accepted and recorded in §8. One deliberate
  non-drift: use an **inline viewport** (`Viewport::Inline`), not the alt screen, so
  the final summary frame stays in terminal scrollback after exit, as in Go.
  The two bubbles widgets are still **written**, not picked: a tick-driven spinner
  (loading phase) and a scrollable plan view with bubbles' keymap (pgup/pgdn, b/f,
  u/d, j/k).
- **B. Async — DECIDED (2026-10-26): no tokio.** `std::thread::scope` + `std::sync::mpsc`
  + blocking `ureq` (30s timeout, matching Go's `net/http`) + `jwalk`. Matches the
  rest of the workspace, fewer deps; the Go binary's concurrency is semaphore-bounded
  goroutines + result channels, which maps 1:1 onto threads + mpsc.
- **C. Errors — DECIDED (2026-10-26): `anyhow` + explicit exit-code mapping.**
  `anyhow` as in `cargo-target-clean` (this binary is orchestration glue, no error
  hierarchy to consume). `main` does **not** return `Result`: it computes the exit
  code per §4.11's matrix (prune failures → 0, declined tracking fix → 1, post-TUI
  load errors → 0, pre-TUI errors → 1) and calls `process::exit`; the matrix is
  pinned by a test in M5/M6.
- **D. Scope of `init` — DECIDED (2026-10-26): commented YAML template string.**
  Replace the 60-LOC anonymous-struct marshal with a commented template in the
  binary (`$HOME` substituted). Keep the exists-check, dir 0755 / file 0644 modes,
  and the `Config file created at %s` message. §8 item 3 records that the emitted
  bytes change (comments added, optional forgejo/token keys now visible).
- **E. exec's `.gitmodules` quirk (§4.8) — DECIDED (2026-10-26): fix lookup, and
  `exec` starts seeing linked worktrees.** Nearest `.gitmodules` per repo replaces
  last-file-wins (§8 item 4); discovery now accepts `.git` *files* (linked
  worktrees) in addition to `.git` directories, so `exec` visits worktrees it never
  visited in Go (also §8 item 4 — note this changes which repos the §6.3 exec
  comparison covers; the harness must compare on a tree without linked worktrees
  or filter worktree paths). Keep the basename-vs-path match quirk and
  `CombinedOutput` semantics as-is.
- **F. Plan-file JSON contract — DECIDED (2026-10-26): idiomatic serde format,
  with load-time migration from Go files.** The plan file gets a clean Rust wire
  format: snake_case field names, string-tagged enums, standard serde presence,
  2-space pretty JSON, atomic save, 0600. `load` parses the new format first and
  falls back to the legacy Go format (below), migrating in memory; `save` always
  writes the new format, so a pending Go plan migrates on its first save. No
  standalone migration command. The legacy Go wire format (kept for the fallback
  parser) is:
  - wrapper `{"actions": [ … ]}`, `MarshalIndent` with **2 spaces**;
  - **PascalCase** field names, because Go exports them: `Type`, `Repo`, `LocalPath`,
    `CurrentPath`, `ExpectedPath`, `AlreadyInPlace`, `WorktreePath`, `MainRepoPath`,
    `Branch`, `Merged`; inside `Repo`: `Name`, `Owner`, `IsFork`, `IsArchived`,
    `IsMirror`, `CloneURL`, `Source`;
  - enums as **ints** (`Update`=0, `Move`=1, `Clone`=2, `PruneWorktree`=3;
    `GitHub`=0, `Forgejo`=1) — the fallback parser must use int variants, not
    string tags;
  - **no `omitempty`**: every field is always present, including empty strings and
    `false` (confirmed against the real `plans/.json` on disk);
  - in the fallback parser: `#[serde(rename_all = "PascalCase")]`, int enums via
    `#[serde(rename = "0")]`-style variants, and **no**
    `skip_serializing_if` anywhere (every field is always present).
  M4 pins it with a legacy-load test (committed Go-written plan JSON parses and
  migrates) plus a new-format round-trip test.
- **G. git2 vs git CLI — DECIDED (2026-10-26): shell out everywhere, no git2.**
  Every git operation is a `git` subprocess (Go does exactly this; no library in
  the tree), so stderr text and §4.10's substring contracts are preserved for
  free and the crate stays pure-Rust. No git2 dependency at all (§3's Git row).
- **H. TUI event-loop architecture (new, prerequisite for M5a) — DECIDED
  (2026-10-26): pure `SyncApp` state machine + thin single-loop runtime.** One
  thread reads crossterm keys, workers feed an `mpsc`, ticks come off an interval,
  resize joins the same channel — all consumed by one blocking loop. No
  `crossterm::event::EventStream`, no async (per B). `SyncApp` owns no terminal:
  `handle_key`/`handle_msg` take plain enums and return state transitions plus
  commands, mirroring bubbletea's `Model` contract so the 8 `tui/sync_test` ports
  run headless. Cancel semantics pinned to Go: `q` stops *scheduling* new actions
  while in-flight git subprocesses run to completion.
- **I. Prune with no known remotes (§4.13) — DECIDED (2026-10-26): add the guard
  now, deliberate §8 deviation.** When the set of known remote URLs is empty
  (no `github_owners`, Forgejo unconfigured or empty), prune refuses to build
  prune actions: the run reports the refusal plus the count of would-be orphans
  and exits **1** (informational, exit 0, under `--dry-run`). No new CLI flag —
  the surface stays as §1. Implemented with the prune flow in M3, with tests
  (empty known-set + orphans → refuse; configured remotes → unchanged Go behavior).

## 8. Out of scope, and deviations we are accepting

No new features, no config schema changes, no output redesign, no `sync --json`.
Anything tempting discovered along the way goes into a follow-up issue list rather
than into the port.

Accepted deviations from the Go binary (all deliberate, all documented in the
README at cutover):

1. **CLI stderr wording** — clap does not print usage on runtime errors the way
   cobra does. Flag names, accepted forms, and exit codes stay as in §1/§4.11.
2. **Parallel dirty check** — faster, same result set (§4.7).
3. **`init` output bytes** — commented template instead of a marshalled anonymous
   struct (decision D).
4. **`exec` discovery** — nearest `.gitmodules` per repo instead of
   last-file-wins, and linked worktrees (`.git` files) are now visited, unlike Go
   (decision E). The M6/§6.3 exec comparison runs on a tree without linked
   worktrees, or filters worktree paths out of the diff.
5. **Internal, not observable:** the declined-action string match replaced by an
   action id (§4.10), and the dead `byName` index / `ConfirmModel` not ported (§2).
6. **TUI visuals** — idiomatic ratatui widgets replace Bubbletea's hand-built
   render-to-string frames: spacing, borders, and colors differ from the Go TUI
   (decision A). Keymap, phase flow, and the inline (scrollback-preserving)
   rendering mode are kept.
7. **Plan-file JSON format** — idiomatic serde (snake_case, string enums) replaces
   Go's PascalCase/int wire format; `load` accepts the legacy Go format and migrates
   in memory, `save` writes only the new format (decision F).
8. **Prune with no known remotes** — guarded, unlike Go: prune refuses (exit 1,
   explanatory message, count of would-be orphans) when no remote is configured and
   orphan candidates exist; `--dry-run` reports the refusal informationally. No new
   CLI flag (decision I). This is the one place the port is deliberately *safer*
   than the Go binary, and the only place `sync`'s exit code differs from Go.

