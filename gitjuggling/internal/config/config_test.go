package config

import (
	"testing"

	"gopkg.in/yaml.v3"
)

func TestParseMinimalConfig(t *testing.T) {
	input := `
workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
`
	cfg, err := parseYAML(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if cfg.DefaultWorkspace != "" {
		t.Errorf("expected empty DefaultWorkspace, got %q", cfg.DefaultWorkspace)
	}

	ws, ok := cfg.Workspaces["personal"]
	if !ok {
		t.Fatal("expected workspace 'personal' to exist")
	}

	if ws.Root != "/home/user/dev" {
		t.Errorf("expected Root /home/user/dev, got %q", ws.Root)
	}

	if len(ws.GitHubOwners) != 1 || ws.GitHubOwners[0] != "vrischmann" {
		t.Errorf("expected GitHubOwners [vrischmann], got %v", ws.GitHubOwners)
	}

	if ws.ForgejoURL != "" {
		t.Errorf("expected empty ForgejoURL, got %q", ws.ForgejoURL)
	}
	if ws.ForgejoUser != "" {
		t.Errorf("expected empty ForgejoUser, got %q", ws.ForgejoUser)
	}
	if ws.ForgejoToken != "" {
		t.Errorf("expected empty ForgejoToken, got %q", ws.ForgejoToken)
	}
	if ws.LocalScanRootDir != "" {
		t.Errorf("expected empty LocalScanRootDir, got %q", ws.LocalScanRootDir)
	}

	if ws.Rules.Base != "/home/user/dev/repos" {
		t.Errorf("expected Rules.Base /home/user/dev/repos, got %q", ws.Rules.Base)
	}
	if ws.Rules.Forks != "" {
		t.Errorf("expected empty Rules.Forks, got %q", ws.Rules.Forks)
	}
	if ws.Rules.Archived != "" {
		t.Errorf("expected empty Rules.Archived, got %q", ws.Rules.Archived)
	}
}

func TestParseFullConfig(t *testing.T) {
	input := `
default_workspace: personal

workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    forgejo_url: https://git.example.com
    forgejo_user: vincent
    forgejo_token: "op://vault/item/field"
    local_scan_root: /home/user/dev
    rules:
      base: /home/user/dev/repos
      forks: /home/user/dev/forks
      archived: /home/user/dev/archived

  work:
    root: /home/user/work
    github_owners: [MyOrg]
    rules:
      base: /home/user/work/repos
`
	cfg, err := parseYAML(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if cfg.DefaultWorkspace != "personal" {
		t.Errorf("expected DefaultWorkspace 'personal', got %q", cfg.DefaultWorkspace)
	}

	if len(cfg.Workspaces) != 2 {
		t.Fatalf("expected 2 workspaces, got %d", len(cfg.Workspaces))
	}

	personal := cfg.Workspaces["personal"]
	if personal.ForgejoURL != "https://git.example.com" {
		t.Errorf("expected ForgejoURL 'https://git.example.com', got %q", personal.ForgejoURL)
	}
	if personal.ForgejoUser != "vincent" {
		t.Errorf("expected ForgejoUser 'vincent', got %q", personal.ForgejoUser)
	}
	if personal.Rules.Forks != "/home/user/dev/forks" {
		t.Errorf("expected Rules.Forks '/home/user/dev/forks', got %q", personal.Rules.Forks)
	}
	if personal.Rules.Archived != "/home/user/dev/archived" {
		t.Errorf("expected Rules.Archived '/home/user/dev/archived', got %q", personal.Rules.Archived)
	}

	work := cfg.Workspaces["work"]
	if len(work.GitHubOwners) != 1 || work.GitHubOwners[0] != "MyOrg" {
		t.Errorf("expected GitHubOwners [MyOrg], got %v", work.GitHubOwners)
	}
	if work.ForgejoURL != "" {
		t.Errorf("expected empty ForgejoURL, got %q", work.ForgejoURL)
	}
}

func TestGetWorkspace(t *testing.T) {
	input := `
default_workspace: personal

workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
`
	cfg, err := parseYAML(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	// Explicit name
	ws, err := cfg.GetWorkspace("personal")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if ws.Root != "/home/user/dev" {
		t.Errorf("expected Root /home/user/dev, got %q", ws.Root)
	}

	// Default fallback
	ws, err = cfg.GetWorkspace("")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if ws.Root != "/home/user/dev" {
		t.Errorf("expected Root /home/user/dev, got %q", ws.Root)
	}

	// Missing workspace
	_, err = cfg.GetWorkspace("nonexistent")
	if err == nil {
		t.Fatal("expected error for missing workspace")
	}
}

func TestLocalScanRootDefault(t *testing.T) {
	input := `
workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
`
	cfg, err := parseYAML(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	ws := cfg.Workspaces["personal"]
	if got := ws.LocalScanRoot(); got != "/home/user/dev" {
		t.Errorf("expected LocalScanRoot /home/user/dev, got %q", got)
	}
}

func TestLocalScanRootOverride(t *testing.T) {
	input := `
workspace:
  personal:
    root: /home/user/dev
    local_scan_root: /home/user/dev/deeper
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
`
	cfg, err := parseYAML(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	ws := cfg.Workspaces["personal"]
	if got := ws.LocalScanRoot(); got != "/home/user/dev/deeper" {
		t.Errorf("expected LocalScanRoot /home/user/dev/deeper, got %q", got)
	}
}

// parseYAML is a test helper that parses a YAML string into a Config.
func parseYAML(input string) (*Config, error) {
	var cfg Config
	if err := yaml.Unmarshal([]byte(input), &cfg); err != nil {
		return nil, err
	}
	return &cfg, nil
}
