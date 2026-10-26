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
    ├── git.rs          # read ops (git2) + network read ops (ls-remote via CLI)
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

Decision E: replicate the quirks (parity) or fix them. Recommend fixing the
`.gitmodules` lookup (nearest file, per repo) and *naming* the worktree blindness
in §8 as a known deviation, since changing which repos `exec` visits changes which
repos the §6.3 comparison covers. Keep `CombinedOutput` semantics.

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
loss. That is current Go behavior; the port should reproduce it and record a
follow-up issue proposing a guard (decision I).

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
| **M4** | `state` (JSON **to the §7F contract**, atomic, 0600) + `exec` (all git mutations, askpass, bounded runner, dry-run) | M | port execute_test, syncstate_test + a **byte-fixture test**: Go-written plan JSON loads in Rust and re-serializes identically + a temp-repo end-to-end: clone/move/update/prune against a local bare "origin" |
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

## 7. Decisions I need from you

- **A. TUI rendering style.** `ratatui` 0.30.2 is the only serious option (with
  `crossterm` 0.29). Sub-decision: reproduce Bubbletea's *render-to-string* style
  (hand-built styled line buffers → closest visual parity, most boilerplate) vs
  idiomatic ratatui widgets (`Paragraph`/`List`/`Gauge`; cleaner, but spacing and
  color drift). Either way two bubbles widgets have to be **written**, not picked:
  a tick-driven spinner (loading phase) and a scrollable viewport with bubbles'
  keymap (pgup/pgdn, b/f, u/d, j/k) for the plan view.
- **B. Async.** Recommend **no tokio**: `std::thread::scope` + `mpsc` +
  blocking `ureq` + `jwalk`. Matches the rest of the workspace, fewer deps.
- **C. Errors.** Recommend `anyhow` (as `cargo-target-clean`), not `onlyerror`
  (as `git-stacked`) — this binary is all orchestration glue. But note: `main` must
  **not** return `Result` naively, because §4.11's exit-code matrix needs failures
  that Go reports with exit 0.
- **D. Scope of `init`.** Keep it as-is, or replace the 60-LOC anonymous-struct YAML
  dump with a commented YAML template string. Recommend the template string — but
  record in §8 that this changes the bytes `init` emits today (dir 0755, file 0644,
  `omitempty` field set, no comments), keeping the exists-check and the file modes.
- **E. exec's `.gitmodules` quirk (§4.8).** Replicate or fix; if fixed, also decide
  whether `exec` should start seeing linked worktrees (it cannot today).
- **F. Plan-file JSON contract.** Recommend byte-compatibility so a Go-started plan
  can be finished by the Rust binary during the transition. That is only meaningful
  if the shape is spelled out, and Go's structs have no `json` tags, so today's wire
  format is:
  - wrapper `{"actions": [ … ]}`, `MarshalIndent` with **2 spaces**;
  - **PascalCase** field names, because Go exports them: `Type`, `Repo`, `LocalPath`,
    `CurrentPath`, `ExpectedPath`, `AlreadyInPlace`, `WorktreePath`, `MainRepoPath`,
    `Branch`, `Merged`; inside `Repo`: `Name`, `Owner`, `IsFork`, `IsArchived`,
    `IsMirror`, `CloneURL`, `Source`;
  - enums as **ints** (`Update`=0, `Move`=1, `Clone`=2, `PruneWorktree`=3;
    `GitHub`=0, `Forgejo`=1) — serde must not use string tags;
  - **no `omitempty`**: every field is always present, including empty strings and
    `false` (confirmed against the real `plans/.json` on disk);
  - in serde: `#[serde(rename_all = "PascalCase")]`, untagged/int enums via
    `#[serde(rename = "0")]`-style or a custom serializer, and **no**
    `skip_serializing_if` anywhere.
  Pin it with a byte-fixture test in M4 (load a Go-written file, re-serialize,
  compare bytes).
- **G. git2 vs git CLI.** Recommend: shell out for everything that mutates
  (`stash`, `pull --rebase`, `clone`, `push -u`, `checkout`, `branch -d`,
  `worktree remove`, `ls-remote`) so stderr text stays identical — §4.10's
  substring checks depend on it — and also for `status --porcelain` and
  `remote get-url` (§4.7). `git2` then earns its place only for
  `rev-parse`/`symbolic-ref`-style reads, if at all; consider dropping it entirely.
- **H. TUI event-loop architecture (new, prerequisite for M5a).** How crossterm key
  events, worker-thread `ActionResult`s, spinner ticks and `Resize` merge into one
  loop, and how the state machine stays a pure struct so the 8 `tui/sync_test` ports
  run headless. Also fixes cancel semantics: `q` stops *scheduling* new actions while
  in-flight git subprocesses run to completion (Go's behavior). Recommend:
  `crossterm::event::EventStream`-free design — one thread reading keys, workers on
  an `mpsc`, a tick interval, all selected in a single blocking loop — paired with a
  `SyncApp` struct whose `handle_key`/`handle_msg` return state transitions plus
  commands, mirroring bubbletea's `Model` contract so the ported tests transfer
  almost line for line.
- **I. Prune with no known remotes (§4.13).** With both remotes unconfigured, Go
  offers to delete every local repo. Reproduce for parity and file a follow-up for a
  guard, or add the guard now as a deliberate deviation? Recommend parity + issue.

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
4. **`exec` `.gitmodules` lookup** — nearest-file per repo instead of
   last-file-wins (decision E).
5. **Internal, not observable:** the declined-action string match replaced by an
   action id (§4.10), and the dead `byName` index / `ConfirmModel` not ported (§2).

