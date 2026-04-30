use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub default_workspace: Option<String>,
    pub workspace: HashMap<String, Workspace>,
}

#[derive(Debug, Deserialize)]
pub struct Workspace {
    pub root: PathBuf,
    #[serde(default)]
    pub github_owners: Vec<String>,
    pub forgejo_url: Option<String>,
    pub forgejo_user: Option<String>,
    pub forgejo_token_cmd: Option<String>,
    #[serde(default)]
    pub local_scan_root: Option<PathBuf>,
    pub rules: Rules,
}

#[derive(Debug, Deserialize)]
pub struct Rules {
    pub base: PathBuf,
    pub forks: Option<PathBuf>,
    pub archived: Option<PathBuf>,
}

impl Config {
    /// Load config from a specific file path.
    pub fn load_from(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file: {}", path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config file: {}", path.display()))?;
        Ok(config)
    }

    /// Load config from the default path (~/.config/gitjuggling/config.toml).
    pub fn load_default() -> Result<Self> {
        let home = dirs_home()?;
        let path = home.join(".config").join("gitjuggling").join("config.toml");
        Self::load_from(&path)
    }

    /// Get a workspace by name, falling back to default_workspace.
    pub fn get_workspace(&self, name: Option<&str>) -> Result<&Workspace> {
        let name = name.or(self.default_workspace.as_deref()).ok_or_else(|| {
            anyhow::anyhow!("no workspace specified and no default_workspace configured")
        })?;

        self.workspace
            .get(name)
            .with_context(|| format!("workspace '{}' not found in config", name))
    }
}

impl Workspace {
    /// Get the local scan root, defaults to the workspace root.
    pub fn local_scan_root(&self) -> &Path {
        self.local_scan_root.as_deref().unwrap_or(&self.root)
    }
}

fn dirs_home() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME environment variable not set")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
[workspace.personal]
root = "/home/user/dev"
github_owners = ["vrischmann"]

[workspace.personal.rules]
base = "/home/user/dev/repos"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.default_workspace.is_none());
        assert!(config.workspace.contains_key("personal"));

        let ws = &config.workspace["personal"];
        assert_eq!(ws.root, PathBuf::from("/home/user/dev"));
        assert_eq!(ws.github_owners, vec!["vrischmann"]);
        assert!(ws.forgejo_url.is_none());
        assert!(ws.forgejo_user.is_none());
        assert!(ws.forgejo_token_cmd.is_none());
        assert!(ws.local_scan_root.is_none());
        assert_eq!(ws.rules.base, PathBuf::from("/home/user/dev/repos"));
        assert!(ws.rules.forks.is_none());
        assert!(ws.rules.archived.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
default_workspace = "personal"

[workspace.personal]
root = "/home/user/dev"
github_owners = ["vrischmann"]
forgejo_url = "https://git.example.com"
forgejo_user = "vincent"
forgejo_token_cmd = "op read 'op://vault/item/field'"
local_scan_root = "/home/user/dev"

[workspace.personal.rules]
base = "/home/user/dev/repos"
forks = "/home/user/dev/forks"
archived = "/home/user/dev/archived"

[workspace.work]
root = "/home/user/work"
github_owners = ["MyOrg"]

[workspace.work.rules]
base = "/home/user/work/repos"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.default_workspace.as_deref(), Some("personal"));
        assert_eq!(config.workspace.len(), 2);

        let personal = &config.workspace["personal"];
        assert_eq!(
            personal.forgejo_url.as_deref(),
            Some("https://git.example.com")
        );
        assert_eq!(personal.forgejo_user.as_deref(), Some("vincent"));
        assert_eq!(
            personal.rules.forks.as_deref(),
            Some(std::path::Path::new("/home/user/dev/forks"))
        );

        let work = &config.workspace["work"];
        assert_eq!(work.github_owners, vec!["MyOrg"]);
        assert!(work.forgejo_url.is_none());
    }

    #[test]
    fn test_get_workspace() {
        let toml = r#"
default_workspace = "personal"

[workspace.personal]
root = "/home/user/dev"
github_owners = ["vrischmann"]

[workspace.personal.rules]
base = "/home/user/dev/repos"
"#;
        let config: Config = toml::from_str(toml).unwrap();

        // Explicit name
        let ws = config.get_workspace(Some("personal")).unwrap();
        assert_eq!(ws.root, PathBuf::from("/home/user/dev"));

        // Default fallback
        let ws = config.get_workspace(None).unwrap();
        assert_eq!(ws.root, PathBuf::from("/home/user/dev"));

        // Missing workspace
        assert!(config.get_workspace(Some("nonexistent")).is_err());
    }

    #[test]
    fn test_local_scan_root_default() {
        let toml = r#"
[workspace.personal]
root = "/home/user/dev"
github_owners = ["vrischmann"]

[workspace.personal.rules]
base = "/home/user/dev/repos"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ws = &config.workspace["personal"];
        assert_eq!(ws.local_scan_root(), PathBuf::from("/home/user/dev"));
    }

    #[test]
    fn test_local_scan_root_override() {
        let toml = r#"
[workspace.personal]
root = "/home/user/dev"
local_scan_root = "/home/user/dev/deeper"
github_owners = ["vrischmann"]

[workspace.personal.rules]
base = "/home/user/dev/repos"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let ws = &config.workspace["personal"];
        assert_eq!(ws.local_scan_root(), PathBuf::from("/home/user/dev/deeper"));
    }
}
