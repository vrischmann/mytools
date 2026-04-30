use crate::config::Workspace;
use crate::discover::LocalRepos;
use crate::remote::RemoteRepo;
use std::path::{Path, PathBuf};

/// A planned action for a single remote repo.
#[derive(Debug)]
pub enum Action {
    /// Repo exists at the correct path → update in place.
    Update {
        repo: RemoteRepo,
        local_path: PathBuf,
    },
    /// Repo exists locally but at the wrong path → needs move.
    Move {
        repo: RemoteRepo,
        current_path: PathBuf,
        expected_path: PathBuf,
    },
    /// Repo doesn't exist locally → needs clone.
    Clone {
        repo: RemoteRepo,
        expected_path: PathBuf,
    },
}

impl Action {
    #[allow(dead_code)]
    pub fn repo(&self) -> &RemoteRepo {
        match self {
            Action::Update { repo, .. } => repo,
            Action::Move { repo, .. } => repo,
            Action::Clone { repo, .. } => repo,
        }
    }

    #[allow(dead_code)]
    pub fn expected_path(&self) -> &Path {
        match self {
            Action::Update { local_path, .. } => local_path,
            Action::Move { expected_path, .. } => expected_path,
            Action::Clone { expected_path, .. } => expected_path,
        }
    }
}

/// Classify a remote repo into its expected local directory based on rules.
///
/// Priority:
/// 1. is_fork → rules.forks (if configured)
/// 2. is_archived → rules.archived (if configured)
/// 3. default → rules.base
pub fn classify_repo(repo: &RemoteRepo, workspace: &Workspace) -> PathBuf {
    let dir = if repo.is_fork {
        workspace.rules.forks.as_deref()
    } else if repo.is_archived {
        workspace.rules.archived.as_deref()
    } else {
        None
    };

    let base_dir = dir.unwrap_or(&workspace.rules.base);
    base_dir.join(format!("{}/{}", repo.owner, repo.name))
}

/// Build a sync plan: determine the action for each remote repo.
pub fn build_plan(
    remote_repos: &[RemoteRepo],
    local: &LocalRepos,
    workspace: &Workspace,
) -> Vec<Action> {
    let mut actions = Vec::new();

    for repo in remote_repos {
        let expected_path = classify_repo(repo, workspace);

        match local.find_by_url(&repo.clone_url) {
            Some(local_path) => {
                if *local_path == expected_path {
                    actions.push(Action::Update {
                        repo: repo.clone(),
                        local_path: local_path.clone(),
                    });
                } else {
                    actions.push(Action::Move {
                        repo: repo.clone(),
                        current_path: local_path.clone(),
                        expected_path,
                    });
                }
            }
            None => {
                actions.push(Action::Clone {
                    repo: repo.clone(),
                    expected_path,
                });
            }
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rules;
    use crate::remote::RemoteSource;

    fn test_workspace() -> Workspace {
        Workspace {
            root: PathBuf::from("/home/user/dev"),
            github_owners: vec!["testowner".to_string()],
            forgejo_url: None,
            forgejo_user: None,
            forgejo_token_cmd: None,
            local_scan_root: None,
            rules: Rules {
                base: PathBuf::from("/home/user/dev/repos"),
                forks: Some(PathBuf::from("/home/user/dev/forks")),
                archived: Some(PathBuf::from("/home/user/dev/archived")),
            },
        }
    }

    fn make_repo(name: &str, owner: &str, is_fork: bool, is_archived: bool) -> RemoteRepo {
        RemoteRepo {
            name: name.to_string(),
            owner: owner.to_string(),
            is_fork,
            is_archived,
            is_mirror: false,
            clone_url: format!("https://github.com/{}/{}.git", owner, name),
            source: RemoteSource::GitHub,
        }
    }

    #[test]
    fn test_classify_base_repo() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);
        let path = classify_repo(&repo, &ws);
        assert_eq!(path, PathBuf::from("/home/user/dev/repos/owner/myrepo"));
    }

    #[test]
    fn test_classify_fork() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", true, false);
        let path = classify_repo(&repo, &ws);
        assert_eq!(path, PathBuf::from("/home/user/dev/forks/owner/myrepo"));
    }

    #[test]
    fn test_classify_archived() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, true);
        let path = classify_repo(&repo, &ws);
        assert_eq!(path, PathBuf::from("/home/user/dev/archived/owner/myrepo"));
    }

    #[test]
    fn test_classify_fork_takes_priority_over_archived() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", true, true);
        let path = classify_repo(&repo, &ws);
        // Fork has higher priority
        assert_eq!(path, PathBuf::from("/home/user/dev/forks/owner/myrepo"));
    }

    #[test]
    fn test_classify_no_forks_dir_falls_back_to_base() {
        let mut ws = test_workspace();
        ws.rules.forks = None;
        let repo = make_repo("myrepo", "owner", true, false);
        let path = classify_repo(&repo, &ws);
        // No forks dir → falls to base
        assert_eq!(path, PathBuf::from("/home/user/dev/repos/owner/myrepo"));
    }

    #[test]
    fn test_build_plan_clone() {
        let ws = test_workspace();
        let local = LocalRepos {
            repos: vec![],
            by_url: Default::default(),
            by_name: Default::default(),
        };
        let repo = make_repo("newrepo", "owner", false, false);
        let actions = build_plan(&[repo], &local, &ws);

        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::Clone { expected_path, .. } => {
                assert_eq!(
                    *expected_path,
                    PathBuf::from("/home/user/dev/repos/owner/newrepo")
                );
            }
            _ => panic!("expected Clone action"),
        }
    }

    #[test]
    fn test_build_plan_update() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);
        let expected = PathBuf::from("/home/user/dev/repos/owner/myrepo");

        let mut by_url = std::collections::HashMap::new();
        by_url.insert("github.com/owner/myrepo".to_string(), expected.clone());

        let local = LocalRepos {
            repos: vec![],
            by_url,
            by_name: Default::default(),
        };

        let actions = build_plan(&[repo], &local, &ws);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::Update { local_path, .. } => {
                assert_eq!(*local_path, expected);
            }
            _ => panic!("expected Update action"),
        }
    }

    #[test]
    fn test_build_plan_move() {
        let ws = test_workspace();
        let repo = make_repo("myrepo", "owner", false, false);

        let wrong_path = PathBuf::from("/home/user/dev/some-other-place/owner/myrepo");
        let mut by_url = std::collections::HashMap::new();
        by_url.insert("github.com/owner/myrepo".to_string(), wrong_path.clone());

        let local = LocalRepos {
            repos: vec![],
            by_url,
            by_name: Default::default(),
        };

        let actions = build_plan(&[repo], &local, &ws);
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::Move {
                current_path,
                expected_path,
                ..
            } => {
                assert_eq!(*current_path, wrong_path);
                assert_eq!(
                    *expected_path,
                    PathBuf::from("/home/user/dev/repos/owner/myrepo")
                );
            }
            _ => panic!("expected Move action"),
        }
    }
}
