//! Durable per-workspace sync-plan state (port of
//! `internal/syncstate/syncstate.go`) — milestone M4a
//! (RUST_REWRITE_PLAN.md §5, §7 decision F).
//!
//! The plan file lives at
//! `<UserConfigDir>/gitjuggling/plans/<workspace>.json`; actions are removed
//! as they succeed and the file is deleted once empty, so a resumed run
//! retries only unfinished work. [`save`] is atomic (temp file in the plans
//! directory, mode 0600, rename) and **always writes the new idiomatic
//! format** (snake_case, string enum tags, 2-space pretty JSON); [`load`]
//! parses the new format first and falls back to the legacy Go wire format,
//! migrating to the new types in memory (§7 decision F — the next [`save`]
//! then persists the migrated plan).
//!
//! The legacy format (kept only for the fallback parser, spelled out in §7
//! decision F): wrapper key `"actions"`, PascalCase field names because Go
//! exports them, enums as ints (`Update`=0…`PruneWorktree`=3, `GitHub`=0 /
//! `Forgejo`=1), no `omitempty` — every field always present, empty strings
//! and `false` included.
//!
//! Deviations from Go, all deliberate:
//!
//! * `load`/`save`/`delete` resolve the config directory through
//!   `dirs::config_dir()` (same resolution as `os.UserConfigDir` on
//!   macOS/Linux, see `config::default_path`); the `*_at` variants take the
//!   config directory as a parameter so tests (and later the TUI) can inject
//!   a sandboxed directory instead of mutating process state.
//! * The legacy int enum tags are decoded with `#[serde(try_from = "u8")]`
//!   rather than the `#[serde(rename = "0")]`-style variants sketched in §7
//!   decision F: serde unit-variant renames only ever match JSON *strings*,
//!   and Go emits bare JSON numbers (`invalid type: integer, expected
//!   variant identifier` otherwise). Unknown ints error, where Go's
//!   `encoding/json` would silently accept any int into the `ActionType`
//!   field — corrupt files are rejected instead of producing a plan the TUI
//!   cannot act on.
//! * Both parse failures are reported together in one error (prefixed with
//!   Go's `parsing saved sync plan` wording) so a corrupt file is
//!   diagnosable without re-running under either format.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::plan::{Action, ActionType};
use crate::remote::{RemoteRepo, Source};

/// The durable portion of a sync operation (Go: `syncstate.Plan`). Actions
/// are removed as they succeed, which makes a later resume retry only
/// unfinished work.
///
/// Serialized in the new plan-file format (§7 decision F): a single
/// `"actions"` array of [`Action`]s (which carry their own snake_case,
/// string-tagged serde contract), pretty-printed with 2-space indent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub actions: Vec<Action>,
}

/// Path of the saved plan for `workspace` (Go: `syncstate.pathFor`):
/// `<UserConfigDir>/gitjuggling/plans/<workspace>.json`.
pub fn path_for(workspace: &str) -> Result<PathBuf> {
    Ok(plans_dir(&config_dir()?).join(format!("{workspace}.json")))
}

/// Reads the saved plan for `workspace` (Go: `syncstate.Load`). A missing
/// plan — or one with no pending actions — is reported as [`None`].
pub fn load(workspace: &str) -> Result<Option<Plan>> {
    load_at(&config_dir()?, workspace)
}

/// Atomically writes `plan` for `workspace`, always in the new format
/// (Go: `syncstate.Save`).
pub fn save(workspace: &str, plan: &Plan) -> Result<()> {
    save_at(&config_dir()?, workspace, plan)
}

/// Removes the saved plan once every pending action has completed (Go:
/// `syncstate.Delete`); removing a missing plan is not an error.
pub fn delete(workspace: &str) -> Result<()> {
    delete_at(&config_dir()?, workspace)
}

/// `<UserConfigDir>` or an injected equivalent (Go: `os.UserConfigDir`).
///
/// `dirs::config_dir()` resolves like `os.UserConfigDir` on macOS/Linux
/// (`$HOME/Library/Application Support`, `$XDG_CONFIG_HOME` else
/// `$HOME/.config`).
fn config_dir() -> Result<PathBuf> {
    dirs::config_dir().context("determining config directory for saved sync plan")
}

/// The `gitjuggling/plans` directory under the config directory.
fn plans_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("gitjuggling").join("plans")
}

fn plan_path(config_dir: &Path, workspace: &str) -> PathBuf {
    plans_dir(config_dir).join(format!("{workspace}.json"))
}

/// [`load`] against an injected config directory (test/TUI seam).
pub fn load_at(config_dir: &Path, workspace: &str) -> Result<Option<Plan>> {
    let data = match fs::read_to_string(plan_path(config_dir, workspace)) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("reading saved sync plan"),
    };
    parse_plan(&data)
}

/// [`save`] against an injected config directory (test/TUI seam).
///
/// Mirrors Go's mechanics: create the plans directory (0755), encode pretty
/// JSON with 2-space indent, `os.CreateTemp` a file *in the same directory*
/// (0600 on Unix), write, chmod 0600, rename over the target (atomic on the
/// same filesystem). A failure removes the temp file, where Go cleans up via
/// `defer os.Remove`.
pub fn save_at(config_dir: &Path, workspace: &str, plan: &Plan) -> Result<()> {
    let dir = plans_dir(config_dir);
    fs::create_dir_all(&dir).context("creating sync plan directory")?;

    // Always the new format (§7 decision F): a pending Go plan migrates on
    // its first save.
    let data = serde_json::to_string_pretty(plan).context("encoding sync plan")?;

    let (mut tmp, tmp_path) = create_temp(&dir).context("creating temporary sync plan")?;
    let saved = write_and_rename(
        &mut tmp,
        &tmp_path,
        &data,
        &plan_path(config_dir, workspace),
    );
    if saved.is_err() {
        // Go: defer os.Remove(tmpName) — best-effort, the save error wins.
        let _ = fs::remove_file(&tmp_path);
    }
    saved
}

/// Go: `os.CreateTemp(dir, ".sync-plan-*")` — exclusively creates
/// `.sync-plan-<random>` (0600 on Unix at open time, like Go) and retries on
/// name collision, returning the open file and its path.
fn create_temp(dir: &Path) -> std::io::Result<(fs::File, PathBuf)> {
    // Seed from pid + clock; an LCG supplies further candidates.
    let mut seed = u64::from(std::process::id())
        ^ SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
    for _ in 0..10_000 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let path = dir.join(format!(".sync-plan-{seed:08x}"));
        match open_temp(&path) {
            Ok(file) => return Ok((file, path)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free sync plan temp file name",
    ))
}

fn open_temp(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Go: write → `tmp.Chmod(0o600)` → close → `os.Rename(tmpName, path)`.
fn write_and_rename(tmp: &mut fs::File, tmp_path: &Path, data: &str, path: &Path) -> Result<()> {
    tmp.write_all(data.as_bytes())
        .context("writing sync plan")?;
    #[cfg(unix)]
    {
        // Created 0600 already on Unix; the explicit chmod mirrors Go's.
        tmp.set_permissions(fs::Permissions::from_mode(0o600))
            .context("setting sync plan permissions")?;
    }
    fs::rename(tmp_path, path).context("saving sync plan")
}

// [`delete`] against an injected config directory (test/TUI seam).
pub fn delete_at(config_dir: &Path, workspace: &str) -> Result<()> {
    if let Err(err) = fs::remove_file(plan_path(config_dir, workspace))
        && err.kind() != std::io::ErrorKind::NotFound
    {
        return Err(err).context("removing saved sync plan");
    }
    Ok(())
}

/// Parses plan JSON: the new format first, falling back to the legacy Go
/// format (§7 decision F). An empty plan is reported as [`None`], matching
/// Go's `len(plan.Actions) == 0 → (nil, nil)`.
fn parse_plan(data: &str) -> Result<Option<Plan>> {
    match serde_json::from_str::<Plan>(data) {
        Ok(plan) => Ok(non_empty(plan)),
        Err(new_err) => {
            let legacy: LegacyPlan = serde_json::from_str(data).map_err(|legacy_err| {
                anyhow!(
                    "parsing saved sync plan: new format: {new_err}; legacy Go format: {legacy_err}"
                )
            })?;
            Ok(non_empty(legacy.migrate()))
        }
    }
}

/// Collapses an empty plan to `None` (Go: `Load` returns nil for no pending
/// actions).
fn non_empty(plan: Plan) -> Option<Plan> {
    (!plan.actions.is_empty()).then_some(plan)
}

// ---------------------------------------------------------------------------
// Legacy Go wire format (§7 decision F) — the fallback parser's view.
//
// These types mirror the Go structs as `encoding/json` sees them: PascalCase
// exported field names, int-tagged enums, every field always present on
// write. On read, missing fields decode to Go's zero values (`#[serde(default)]`).
// They are never serialized: `save` only ever writes the new format.
// ---------------------------------------------------------------------------

/// Go: `syncplan.Plan` — note the lowercase `actions` key (the one JSON tag
/// Go names explicitly), hence no PascalCase rename here.
#[derive(Debug, Deserialize)]
struct LegacyPlan {
    #[serde(default)]
    actions: Vec<LegacyAction>,
}

impl LegacyPlan {
    /// Migrates the legacy plan to the new types in memory (§7 decision F).
    fn migrate(self) -> Plan {
        Plan {
            actions: self
                .actions
                .into_iter()
                .map(LegacyAction::migrate)
                .collect(),
        }
    }
}

/// Go: `syncplan.Action` (`Type` needs an explicit rename: PascalCase of
/// `action_type` would be `ActionType`).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyAction {
    #[serde(rename = "Type", default)]
    action_type: LegacyActionType,
    /// Go: `*remote.RemoteRepo` (always set by Go's writer).
    #[serde(default)]
    repo: LegacyRemoteRepo,
    #[serde(default)]
    local_path: String,
    #[serde(default)]
    current_path: String,
    #[serde(default)]
    expected_path: String,
    #[serde(default)]
    already_in_place: bool,
    #[serde(default)]
    worktree_path: String,
    #[serde(default)]
    main_repo_path: String,
    #[serde(default)]
    branch: String,
    #[serde(default)]
    merged: bool,
}

impl LegacyAction {
    fn migrate(self) -> Action {
        Action {
            action_type: match self.action_type {
                LegacyActionType::Update => ActionType::Update,
                LegacyActionType::Move => ActionType::Move,
                LegacyActionType::Clone => ActionType::Clone,
                LegacyActionType::PruneWorktree => ActionType::PruneWorktree,
            },
            repo: self.repo.migrate(),
            local_path: PathBuf::from(self.local_path),
            current_path: PathBuf::from(self.current_path),
            expected_path: PathBuf::from(self.expected_path),
            already_in_place: self.already_in_place,
            worktree_path: PathBuf::from(self.worktree_path),
            main_repo_path: PathBuf::from(self.main_repo_path),
            branch: self.branch,
            merged: self.merged,
        }
    }
}

/// Go: `syncplan.ActionType` (`iota`-based int). Defaults to `Update` (0),
/// Go's zero value.
#[derive(Debug, Default, Deserialize)]
#[serde(try_from = "u8")]
enum LegacyActionType {
    #[default]
    Update,
    Move,
    Clone,
    PruneWorktree,
}

impl TryFrom<u8> for LegacyActionType {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Update),
            1 => Ok(Self::Move),
            2 => Ok(Self::Clone),
            3 => Ok(Self::PruneWorktree),
            other => Err(format!("unknown action type {other}")),
        }
    }
}

/// Go: `remote.RemoteRepo`.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LegacyRemoteRepo {
    #[serde(default)]
    name: String,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    is_fork: bool,
    #[serde(default)]
    is_archived: bool,
    #[serde(default)]
    is_mirror: bool,
    /// Explicit rename: PascalCase of `clone_url` would be `CloneUrl`, but
    /// the Go field is `CloneURL`.
    #[serde(rename = "CloneURL", default)]
    clone_url: String,
    #[serde(default)]
    source: LegacySource,
}

impl LegacyRemoteRepo {
    fn migrate(self) -> RemoteRepo {
        RemoteRepo {
            name: self.name,
            owner: self.owner,
            is_fork: self.is_fork,
            is_archived: self.is_archived,
            is_mirror: self.is_mirror,
            clone_url: self.clone_url,
            source: match self.source {
                LegacySource::GitHub => Source::GitHub,
                LegacySource::Forgejo => Source::Forgejo,
            },
        }
    }
}

/// Go: `remote.RemoteSource` (`SourceGitHub` = 0). Defaults to `GitHub`,
/// Go's zero value.
#[derive(Debug, Default, Deserialize)]
#[serde(try_from = "u8")]
enum LegacySource {
    #[default]
    GitHub,
    Forgejo,
}

impl TryFrom<u8> for LegacySource {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::GitHub),
            1 => Ok(Self::Forgejo),
            other => Err(format!("unknown remote source {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// A Go-written plan file (hand-written from the §7 decision F wire
    /// format: PascalCase fields, int enums, every field present), covering
    /// all four action types, both sources, and the fork/archived flags.
    const LEGACY_FIXTURE: &str = include_str!("../tests/fixtures/plan-legacy-go.json");

    /// The plan from Go's TestSaveLoadAndDelete (Clone of a Forgejo repo).
    fn test_plan() -> Plan {
        Plan {
            actions: vec![Action {
                action_type: ActionType::Clone,
                repo: RemoteRepo {
                    name: "repo".into(),
                    owner: "owner".into(),
                    is_fork: false,
                    is_archived: false,
                    is_mirror: false,
                    clone_url: "https://example.test/repo.git".into(),
                    source: Source::Forgejo,
                },
                expected_path: PathBuf::from("/tmp/repo"),
                ..Action::default()
            }],
        }
    }

    fn write_plan_file(config_dir: &Path, workspace: &str, data: &str) {
        let dir = plans_dir(config_dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{workspace}.json")), data).unwrap();
    }

    // Port of syncstate_test.go TestSaveLoadAndDelete (with the config
    // directory injected where Go uses t.Setenv("XDG_CONFIG_HOME", …)).
    #[test]
    fn test_save_load_and_delete() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path();

        let plan = test_plan();
        save_at(config_dir, "personal", &plan).unwrap();
        let loaded = load_at(config_dir, "personal").unwrap().unwrap();
        assert_eq!(loaded, plan);

        delete_at(config_dir, "personal").unwrap();
        assert_eq!(load_at(config_dir, "personal").unwrap(), None);
    }

    // §7 decision F / M4: the on-disk new format is pinned byte-for-byte
    // (snake_case, string enum tags, every field present, 2-space indent),
    // and serializing a re-parsed plan reproduces the exact bytes. Also
    // pins the save mechanics: mode 0600 and no temp file left behind.
    #[test]
    fn test_save_writes_new_format_byte_stable() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path();

        let plan = test_plan();
        save_at(config_dir, "personal", &plan).unwrap();

        let expected = r#"{
  "actions": [
    {
      "type": "clone",
      "repo": {
        "name": "repo",
        "owner": "owner",
        "is_fork": false,
        "is_archived": false,
        "is_mirror": false,
        "clone_url": "https://example.test/repo.git",
        "source": "forgejo"
      },
      "local_path": "",
      "current_path": "",
      "expected_path": "/tmp/repo",
      "already_in_place": false,
      "worktree_path": "",
      "main_repo_path": "",
      "branch": "",
      "merged": false
    }
  ]
}"#;
        let path = plans_dir(config_dir).join("personal.json");
        let on_disk = fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk, expected);

        let reparsed: Plan = serde_json::from_str(&on_disk).unwrap();
        assert_eq!(reparsed, plan);
        assert_eq!(
            serde_json::to_string_pretty(&reparsed).unwrap(),
            on_disk,
            "round-trip must be byte-stable"
        );

        #[cfg(unix)]
        {
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "saved plan must be 0600");
        }
        // The temp file was renamed into place, not left behind.
        assert_eq!(fs::read_dir(plans_dir(config_dir)).unwrap().count(), 1);
    }

    // M4 legacy-load test: a Go-written plan file parses via the fallback
    // and migrates in memory to the new types (§7 decision F).
    #[test]
    fn test_load_legacy_go_plan_migrates() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path();
        write_plan_file(config_dir, "personal", LEGACY_FIXTURE);

        let plan = load_at(config_dir, "personal")
            .unwrap()
            .expect("legacy plan should load");

        let want = vec![
            Action {
                action_type: ActionType::Clone,
                repo: RemoteRepo {
                    name: "tools".into(),
                    owner: "acme".into(),
                    is_fork: true,
                    is_archived: false,
                    is_mirror: false,
                    clone_url: "git@github.com:acme/tools.git".into(),
                    source: Source::GitHub,
                },
                expected_path: PathBuf::from("/home/user/workspaces/forks/tools"),
                ..Action::default()
            },
            Action {
                action_type: ActionType::Update,
                repo: RemoteRepo {
                    name: "api".into(),
                    owner: "acme".into(),
                    is_fork: false,
                    is_archived: false,
                    is_mirror: false,
                    clone_url: "https://github.com/acme/api.git".into(),
                    source: Source::GitHub,
                },
                local_path: PathBuf::from("/home/user/workspaces/acme/api"),
                expected_path: PathBuf::from("/home/user/workspaces/acme/api"),
                already_in_place: true,
                ..Action::default()
            },
            Action {
                action_type: ActionType::Move,
                repo: RemoteRepo {
                    name: "infra".into(),
                    owner: "acme".into(),
                    is_fork: false,
                    is_archived: true,
                    is_mirror: false,
                    clone_url: "https://forgejo.example.test/acme/infra.git".into(),
                    source: Source::Forgejo,
                },
                current_path: PathBuf::from("/home/user/workspaces/acme/infra"),
                expected_path: PathBuf::from("/home/user/workspaces/archived/infra"),
                ..Action::default()
            },
            Action {
                action_type: ActionType::PruneWorktree,
                repo: RemoteRepo {
                    name: "web".into(),
                    owner: "acme".into(),
                    is_fork: false,
                    is_archived: false,
                    is_mirror: false,
                    clone_url: "https://github.com/acme/web.git".into(),
                    source: Source::GitHub,
                },
                worktree_path: PathBuf::from(
                    "/home/user/workspaces/acme/web/worktrees/feature-api",
                ),
                main_repo_path: PathBuf::from("/home/user/workspaces/acme/web"),
                branch: "feature-api".into(),
                merged: true,
                ..Action::default()
            },
        ];
        assert_eq!(plan.actions, want);

        // Decision F: a pending Go plan migrates on its first save — the
        // file is rewritten in the new format only, and reloads identically.
        save_at(config_dir, "personal", &plan).unwrap();
        let on_disk = fs::read_to_string(plans_dir(config_dir).join("personal.json")).unwrap();
        assert!(
            on_disk.contains("\"type\": \"prune_worktree\""),
            "{on_disk}"
        );
        assert!(!on_disk.contains("\"Type\""), "{on_disk}");
        assert_eq!(load_at(config_dir, "personal").unwrap(), Some(plan));
    }

    // The fallback parser rejects int tags outside the Go enum range
    // (Go would silently accept any int; we error instead — module doc).
    #[test]
    fn test_load_legacy_unknown_action_type_rejected() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path();
        let broken = LEGACY_FIXTURE.replace("\"Type\": 2,", "\"Type\": 9,");
        assert_ne!(broken, LEGACY_FIXTURE);
        write_plan_file(config_dir, "personal", &broken);

        let err = load_at(config_dir, "personal").unwrap_err().to_string();
        assert!(err.contains("unknown action type 9"), "{err}");
    }

    // Go: a missing plan loads as (nil, nil).
    #[test]
    fn test_load_missing_file_is_none() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(load_at(tmp.path(), "personal").unwrap(), None);
    }

    // Go: len(plan.Actions) == 0 loads as (nil, nil), even though the file
    // exists (and Save of an empty plan is still written, new-format).
    #[test]
    fn test_load_empty_actions_is_none() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path();
        save_at(config_dir, "personal", &Plan { actions: vec![] }).unwrap();
        assert_eq!(
            fs::read_to_string(plans_dir(config_dir).join("personal.json")).unwrap(),
            "{\n  \"actions\": []\n}"
        );
        assert_eq!(load_at(config_dir, "personal").unwrap(), None);
    }

    // Go: Delete of a missing plan is not an error.
    #[test]
    fn test_delete_missing_plan_is_ok() {
        let tmp = TempDir::new().unwrap();
        assert!(delete_at(tmp.path(), "personal").is_ok());
    }

    // A file neither parser accepts is an error carrying Go's
    // `parsing saved sync plan` wording.
    #[test]
    fn test_load_unparsable_plan_errors() {
        let tmp = TempDir::new().unwrap();
        write_plan_file(tmp.path(), "personal", "not json");
        let err = load_at(tmp.path(), "personal").unwrap_err().to_string();
        assert!(err.contains("parsing saved sync plan"), "{err}");
    }

    // pathFor resolution (dirs::config_dir resolves like os.UserConfigDir
    // on macOS/Linux — config.rs::test_default_path for the same seam).
    #[test]
    fn test_path_for() {
        let path = path_for("personal").unwrap();
        assert!(path.is_absolute(), "path {path:?} is not absolute");
        assert!(
            path.ends_with("gitjuggling/plans/personal.json"),
            "path {path:?}"
        );
    }
}
