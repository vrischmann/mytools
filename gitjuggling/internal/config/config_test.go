package config

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/require"
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
	require.NoError(t, err)

	require.Empty(t, cfg.DefaultWorkspace)

	ws, ok := cfg.Workspaces["personal"]
	require.True(t, ok, "expected workspace 'personal' to exist")

	require.Equal(t, "/home/user/dev", ws.Root)
	require.Equal(t, []string{"vrischmann"}, ws.GitHubOwners)

	require.Empty(t, ws.ForgejoURL)
	require.Empty(t, ws.ForgejoUser)
	require.Empty(t, ws.ForgejoToken)

	require.Equal(t, "/home/user/dev/repos", ws.Rules.Base)
	require.Empty(t, ws.Rules.Forks)
	require.Empty(t, ws.Rules.Archived)
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
	require.NoError(t, err)

	require.Equal(t, "personal", cfg.DefaultWorkspace)
	require.Len(t, cfg.Workspaces, 2)

	personal := cfg.Workspaces["personal"]
	require.Equal(t, "https://git.example.com", personal.ForgejoURL)
	require.Equal(t, "vincent", personal.ForgejoUser)
	require.Equal(t, "/home/user/dev/forks", personal.Rules.Forks)
	require.Equal(t, "/home/user/dev/archived", personal.Rules.Archived)

	work := cfg.Workspaces["work"]
	require.Equal(t, []string{"MyOrg"}, work.GitHubOwners)
	require.Empty(t, work.ForgejoURL)
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
	require.NoError(t, err)

	// Explicit name
	ws, err := cfg.GetWorkspace("personal")
	require.NoError(t, err)
	require.Equal(t, "/home/user/dev", ws.Root)

	// Default fallback
	ws, err = cfg.GetWorkspace("")
	require.NoError(t, err)
	require.Equal(t, "/home/user/dev", ws.Root)

	// Missing workspace
	_, err = cfg.GetWorkspace("nonexistent")
	require.Error(t, err)
}

func TestParseConfigWithIgnore(t *testing.T) {
	input := `
workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    ignore:
      - dotfiles
      - "temp-*"
    rules:
      base: /home/user/dev/repos
`
	cfg, err := parseYAML(input)
	require.NoError(t, err)

	ws := cfg.Workspaces["personal"]
	require.Equal(t, []string{"dotfiles", "temp-*"}, ws.Ignore)
}

func TestIsIgnored(t *testing.T) {
	ws := &Workspace{
		Ignore: []string{"dotfiles", "temp-*", "test?repo"},
	}

	tests := []struct {
		name string
		want bool
	}{
		// Exact match
		{"dotfiles", true},
		{"Dotfiles", true}, // case-insensitive
		{"my-dotfiles", false},

		// Glob pattern
		{"temp-foo", true},
		{"temp-bar-baz", true},
		{"TEMP-xyz", true}, // case-insensitive
		{"temp", false},    // partial match doesn't count

		// Glob with ?
		{"test-repo", true},
		{"testXrepo", true},
		{"testXXrepo", false},

		// No match
		{"myproject", false},
		{"", false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := ws.IsIgnored(tt.name)
			require.Equal(t, tt.want, got)
		})
	}
}

func TestIsIgnoredEmpty(t *testing.T) {
	ws := &Workspace{}
	require.False(t, ws.IsIgnored("anything"))
}

// parseYAML is a test helper that parses a YAML string into a Config.
func parseYAML(input string) (*Config, error) {
	var cfg Config
	if err := yaml.Unmarshal([]byte(input), &cfg); err != nil {
		return nil, err
	}
	return &cfg, nil
}

func TestRulesValidate(t *testing.T) {
	tests := []struct {
		name    string
		rules   Rules
		wantErr string // substring of the error; empty means no error expected
	}{
		{name: "no patterns"},
		{
			name:  "valid capture",
			rules: Rules{Patterns: []PatternRule{{Pattern: `^workspace-(.+)$`, To: "/ws/$1"}}},
		},
		{
			name:  "valid named group",
			rules: Rules{Patterns: []PatternRule{{Pattern: `^workspace-(?P<id>.+)$`, To: "/ws/${id}"}}},
		},
		{
			name:  "valid whole match with no capture groups",
			rules: Rules{Patterns: []PatternRule{{Pattern: `^workspace-.+$`, To: "/ws/$0"}}},
		},
		{
			name:    "to without capture",
			rules:   Rules{Patterns: []PatternRule{{Pattern: `^workspace-(.+)$`, To: "/ws/flat"}}},
			wantErr: "must reference at least one capture",
		},
		{
			name:    "to is only an escaped dollar",
			rules:   Rules{Patterns: []PatternRule{{Pattern: `^x$`, To: "/ws/$$1"}}},
			wantErr: "must reference at least one capture",
		},
		{
			name:    "invalid regex",
			rules:   Rules{Patterns: []PatternRule{{Pattern: `^workspace-[`, To: "/ws/$0"}}},
			wantErr: "invalid regex",
		},
		{
			name:    "out of range group",
			rules:   Rules{Patterns: []PatternRule{{Pattern: `^workspace-(.+)$`, To: "/ws/$2"}}},
			wantErr: "non-existent group",
		},
		{
			name:    "unknown named group",
			rules:   Rules{Patterns: []PatternRule{{Pattern: `^workspace-(.+)$`, To: "/ws/${nope}"}}},
			wantErr: "not a named group",
		},
		{
			name:    "empty pattern",
			rules:   Rules{Patterns: []PatternRule{{Pattern: "", To: "/ws/$0"}}},
			wantErr: "pattern is empty",
		},
		{
			name:    "empty to",
			rules:   Rules{Patterns: []PatternRule{{Pattern: `^x$`, To: ""}}},
			wantErr: "to is empty",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.rules.Validate()
			if tt.wantErr == "" {
				require.NoError(t, err)
				return
			}
			require.Error(t, err)
			require.Contains(t, err.Error(), tt.wantErr)
		})
	}
}

func TestLoadFromRejectsInvalidPattern(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")
	content := `
workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
      patterns:
        - pattern: '^workspace-(.+)$'
          to: /home/user/dev/workspaces/flat
`
	require.NoError(t, os.WriteFile(path, []byte(content), 0o600))

	_, err := LoadFrom(path)
	require.Error(t, err)
	require.Contains(t, err.Error(), "must reference at least one capture")
}
